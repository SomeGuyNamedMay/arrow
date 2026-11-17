import keyword as _keyword
import re as _re

import arrow


HIGHLIGHT_COLORS = {
    "comment": (0.48, 0.53, 0.59, 1),
    "keyword": (0.78, 0.55, 0.96, 2),
    "modifier": (0.78, 0.55, 0.96, 2),
    "function": (0.94, 0.78, 0.42, 3),
    "method": (0.94, 0.78, 0.42, 3),
    "macro": (0.94, 0.78, 0.42, 3),
    "type": (0.42, 0.76, 0.94, 3),
    "class": (0.42, 0.76, 0.94, 3),
    "enum": (0.42, 0.76, 0.94, 3),
    "interface": (0.42, 0.76, 0.94, 3),
    "struct": (0.42, 0.76, 0.94, 3),
    "namespace": (0.42, 0.76, 0.94, 3),
    "lifetime": (0.42, 0.76, 0.94, 3),
    "string": (0.62, 0.82, 0.56, 2),
    "character": (0.62, 0.82, 0.56, 2),
    "number": (0.91, 0.64, 0.42, 2),
    "constant": (0.91, 0.64, 0.42, 2),
    "enum_member": (0.91, 0.64, 0.42, 2),
    "property": (0.75, 0.78, 0.86, 2),
    "parameter": (0.75, 0.78, 0.86, 2),
    "variable": (0.75, 0.78, 0.86, 2),
    "operator": (0.70, 0.74, 0.82, 1),
}


# The bundled parsers are tolerant lexers: they never fail on incomplete code,
# so highlighting and text objects keep working while a buffer is being edited.
# Tokens are (kind, start, end, text) with Unicode character offsets.

_LITERAL_CAPTURES = {
    "comment": "comment",
    "string": "string",
    "character": "character",
    "lifetime": "lifetime",
    "number": "number",
    "operator": "operator",
}


def _build_tree(source, tokens, functions):
    """Nest captured (start, end, capture) tokens inside function nodes."""
    root = arrow.ast_node("module", 0, len(source))
    functions = sorted(set(functions), key=lambda span: (span[0], -span[1]))
    stack = [root]
    next_function = 0
    for start, end, capture in tokens:
        while next_function < len(functions) and functions[next_function][0] <= start:
            function_start, function_end = functions[next_function]
            next_function += 1
            while len(stack) > 1 and stack[-1]["end"] <= function_start:
                stack.pop()
            function_end = min(function_end, stack[-1]["end"])
            if function_start < function_end:
                node = arrow.ast_node("function", function_start, function_end)
                stack[-1]["children"].append(node)
                stack.append(node)
        while len(stack) > 1 and stack[-1]["end"] < end:
            stack.pop()
        stack[-1]["children"].append(arrow.ast_node("token", start, end, capture=capture))
    return root


def _is_constant_name(name):
    return len(name) > 1 and name.isupper()


_RUST_TOKEN = _re.compile(
    r"""
      (?P<comment>//[^\n]*)
    | (?P<block_comment>/\*)
    | (?P<raw_string>[bc]?r(?P<hashes>\#*)")
    | (?P<string>[bc]?"(?:[^"\\]|\\.)*"?)
    | (?P<character>b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^\\'\n])')
    | (?P<lifetime>'[^\W\d]\w*)
    | (?P<number>0[xXoObB]\w+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d[\d_]*)?\w*)
    | (?P<ident>(?:r\#)?[^\W\d]\w*)
    | (?P<operator>[-+*%^!&|=<>@?~]+|/)
    | (?P<open>[(\[{])
    | (?P<close>[)\]}])
    | (?P<semicolon>;)
    """,
    _re.VERBOSE | _re.DOTALL,
)
_RUST_BLOCK_COMMENT_EDGE = _re.compile(r"/\*|\*/")

_RUST_KEYWORDS = frozenset(
    """as async await break const continue crate dyn else enum extern fn for if
    impl in let loop match mod move mut pub ref return self static struct super
    trait type union unsafe use where while yield""".split()
)
_RUST_CONSTANTS = frozenset(("true", "false"))
_RUST_TYPES = frozenset(
    """Self bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize
    f32 f64""".split()
)
_RUST_FUNCTION_MODIFIERS = frozenset(("pub", "const", "async", "unsafe", "extern", "default"))


def _rust_tokens(source):
    tokens = []
    position = 0
    while True:
        match = _RUST_TOKEN.search(source, position)
        if match is None:
            return tokens
        kind = match.lastgroup
        start, end = match.span()
        if kind == "block_comment":
            # Rust block comments nest.
            kind = "comment"
            depth = 1
            while depth:
                edge = _RUST_BLOCK_COMMENT_EDGE.search(source, end)
                if edge is None:
                    end = len(source)
                    break
                end = edge.end()
                depth += 1 if edge.group() == "/*" else -1
        elif kind == "raw_string":
            kind = "string"
            closing = '"' + match.group("hashes")
            close = source.find(closing, end)
            end = len(source) if close < 0 else close + len(closing)
        tokens.append((kind, start, end, source[start:end]))
        position = end


def _rust_item_start(tokens, index):
    """Extend a function start backwards over `pub(crate) const unsafe` etc."""
    start = tokens[index][1]
    index -= 1
    while index >= 0:
        kind, token_start, _, text = tokens[index]
        if kind == "ident" and text in _RUST_FUNCTION_MODIFIERS:
            start = token_start
            index -= 1
        elif kind == "string" and index > 0 and tokens[index - 1][3] == "extern":
            index -= 1
        elif text == ")":
            opening = index - 1
            while opening >= 0 and tokens[opening][0] == "ident":
                opening -= 1
            if opening < 1 or tokens[opening][3] != "(" or tokens[opening - 1][3] != "pub":
                break
            index = opening - 1
        else:
            break
    return start


def _rust_function_end(tokens, index, source_length):
    """Find the end of the function whose name token is at index."""
    depth = 0
    in_body = False
    for position in range(index, len(tokens)):
        kind, start, end, text = tokens[position]
        if kind == "open":
            if depth == 0 and text == "{":
                in_body = True
            depth += 1
        elif kind == "close":
            if depth == 0:
                return start
            depth -= 1
            if depth == 0 and in_body:
                return end
        elif kind == "semicolon" and depth == 0:
            return end
    return source_length


def _rust_identifier_capture(text, previous, following, end):
    if text.startswith("r#"):
        return None
    if text in _RUST_CONSTANTS:
        return "constant"
    if text in _RUST_TYPES:
        return "type"
    if text in _RUST_KEYWORDS:
        return "keyword"
    if (
        following is not None
        and following[0] == "operator"
        and following[1] == end
        and following[3].startswith("!")
        and not following[3].startswith("!=")
    ):
        return "macro"
    if previous is not None and previous[3] == "fn":
        return "function"
    if _is_constant_name(text):
        return "constant"
    if text[0].isupper():
        return "type"
    if following is not None and following[3] == "(":
        return "function"
    return None


def parse_rust(source):
    all_tokens = _rust_tokens(source)
    tokens = [token for token in all_tokens if token[0] != "comment"]
    captured = [
        (start, end, "comment") for kind, start, end, _ in all_tokens if kind == "comment"
    ]
    functions = []
    for index, (kind, start, end, text) in enumerate(tokens):
        previous = tokens[index - 1] if index else None
        following = tokens[index + 1] if index + 1 < len(tokens) else None
        if kind == "ident":
            capture = _rust_identifier_capture(text, previous, following, end)
            if text == "fn" and following is not None and following[0] == "ident":
                functions.append((
                    _rust_item_start(tokens, index),
                    _rust_function_end(tokens, index + 1, len(source)),
                ))
        else:
            capture = _LITERAL_CAPTURES.get(kind)
        if capture is not None:
            captured.append((start, end, capture))
    captured.sort()
    return _build_tree(source, captured, functions)


_PYTHON_TOKEN = _re.compile(
    r"""
      (?P<comment>\#[^\n]*)
    | (?P<string>(?:[rRbBuUfFtT]{1,2})?(?:
          \"\"\"(?:[^\\]|\\.)*?(?:\"\"\"|\Z)
        | '''(?:[^\\]|\\.)*?(?:'''|\Z)
        | "(?:[^"\\\n]|\\.)*"?
        | '(?:[^'\\\n]|\\.)*'?
      ))
    | (?P<number>0[xXoObB]\w+
        |(?:\d[\d_]*(?:\.[\d_]*)?|\.\d[\d_]*)(?:[eE][+-]?\d[\d_]*)?[jJ]?)
    | (?P<ident>[^\W\d]\w*)
    | (?P<operator>[-+*/%^&|=<>!~@]+)
    | (?P<open>[(\[{])
    | (?P<close>[)\]}])
    """,
    _re.VERBOSE | _re.DOTALL,
)

_PYTHON_CONSTANTS = frozenset(("True", "False", "None"))
_PYTHON_KEYWORDS = frozenset(_keyword.kwlist) - _PYTHON_CONSTANTS
_PYTHON_TYPES = frozenset(
    """bool bytearray bytes complex dict float frozenset int list object set
    str tuple type""".split()
)


def _python_identifier_capture(text, previous, following):
    if text in _PYTHON_CONSTANTS:
        return "constant"
    if text in _PYTHON_KEYWORDS:
        return "keyword"
    if previous is not None and previous[0] == "ident":
        if previous[3] == "def":
            return "function"
        if previous[3] == "class":
            return "type"
    if text in _PYTHON_TYPES:
        return "type"
    if _is_constant_name(text):
        return "constant"
    if text[0].isupper():
        return "type"
    if following is not None and following[3] == "(":
        return "function"
    return None


def _python_functions(tokens, logical_lines):
    """Return the span of every `def` block, found by indentation."""
    functions = []
    for line, (index, indent) in enumerate(logical_lines):
        definition = index
        if tokens[definition][3] == "async" and definition + 1 < len(tokens):
            definition += 1
        if (
            tokens[definition][0] != "ident"
            or tokens[definition][3] != "def"
            or definition + 1 >= len(tokens)
            or tokens[definition + 1][0] != "ident"
        ):
            continue
        end_index = len(tokens)
        for later in range(line + 1, len(logical_lines)):
            if logical_lines[later][1] <= indent:
                end_index = logical_lines[later][0]
                break
        functions.append((tokens[index][1], tokens[end_index - 1][2]))
    return functions


def parse_python(source):
    tokens = []
    captured = []
    # (index of the first token, indentation) of each logical line.
    logical_lines = []
    depth = 0
    previous_end = 0
    previous_token_end = 0
    for match in _PYTHON_TOKEN.finditer(source):
        kind = match.lastgroup
        start, end = match.span()
        if kind == "comment":
            captured.append((start, end, "comment"))
            previous_end = end
            continue
        line_start = source.rfind("\n", 0, start) + 1
        if (
            depth == 0
            and previous_token_end <= line_start
            # A backslash outside any token continues the previous line.
            and "\\" not in source[previous_end:line_start]
        ):
            indent = len(source[line_start:start].expandtabs())
            logical_lines.append((len(tokens), indent))
        if kind == "open":
            depth += 1
        elif kind == "close":
            depth = max(0, depth - 1)
        tokens.append((kind, start, end, match.group()))
        previous_end = previous_token_end = end

    for index, (kind, start, end, text) in enumerate(tokens):
        if kind == "ident":
            previous = tokens[index - 1] if index else None
            following = tokens[index + 1] if index + 1 < len(tokens) else None
            capture = _python_identifier_capture(text, previous, following)
        else:
            capture = _LITERAL_CAPTURES.get(kind)
        if capture is not None:
            captured.append((start, end, capture))
    captured.sort()
    return _build_tree(source, captured, _python_functions(tokens, logical_lines))


arrow.register_language(
    "rust",
    ("rs",),
    parser=parse_rust,
    highlight_colors=HIGHLIGHT_COLORS,
    text_object_kinds=("function",),
    language_server={"command": "rust-analyzer", "args": [], "root_markers": ["Cargo.toml"]},
)

arrow.register_language(
    "python",
    ("py", "pyi"),
    parser=parse_python,
    highlight_colors=HIGHLIGHT_COLORS,
    text_object_kinds=("function",),
    language_server={"command": "pyright-langserver", "args": ["--stdio"], "root_markers": ["pyproject.toml", "setup.py", "pyrightconfig.json"]},
)
