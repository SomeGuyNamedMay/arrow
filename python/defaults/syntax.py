import arrow


CAPTURE_COLORS = {
    "comment": (0.48, 0.53, 0.59, 1),
    "comment.documentation": (0.48, 0.53, 0.59, 1),
    "keyword": (0.78, 0.55, 0.96, 2),
    "keyword.control": (0.78, 0.55, 0.96, 2),
    "keyword.operator": (0.78, 0.55, 0.96, 2),
    "function": (0.94, 0.78, 0.42, 3),
    "function.method": (0.94, 0.78, 0.42, 3),
    "function.builtin": (0.94, 0.78, 0.42, 3),
    "function.macro": (0.94, 0.78, 0.42, 3),
    "type": (0.42, 0.76, 0.94, 3),
    "type.builtin": (0.42, 0.76, 0.94, 3),
    "constructor": (0.42, 0.76, 0.94, 3),
    "string": (0.62, 0.82, 0.56, 2),
    "string.special": (0.62, 0.82, 0.56, 2),
    "character": (0.62, 0.82, 0.56, 2),
    "number": (0.91, 0.64, 0.42, 2),
    "constant.builtin": (0.91, 0.64, 0.42, 2),
    "constant": (0.91, 0.64, 0.42, 2),
    "property": (0.75, 0.78, 0.86, 2),
    "variable.parameter": (0.75, 0.78, 0.86, 2),
    "operator": (0.70, 0.74, 0.82, 1),
    "punctuation": (0.70, 0.74, 0.82, 1),
    "punctuation.bracket": (0.70, 0.74, 0.82, 1),
    "punctuation.delimiter": (0.70, 0.74, 0.82, 1),
    "escape": (0.95, 0.63, 0.63, 4),
    "punctuation.special": (0.95, 0.63, 0.63, 4),
}


arrow.register_language(
    "rust",
    "rust",
    ("rs",),
    arrow.builtin_highlights_query("rust"),
    ("function_item", "function_signature_item"),
    CAPTURE_COLORS,
)

arrow.register_language(
    "python",
    "python",
    ("py", "pyi"),
    arrow.builtin_highlights_query("python"),
    ("function_definition",),
    CAPTURE_COLORS,
)
