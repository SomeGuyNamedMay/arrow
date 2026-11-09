import arrow


def selected_range(dot):
    selection = dot.get("selection")
    return selection if selection is not None else (dot["cursor"], dot["cursor"])


def update_focused_dots(update):
    dots = arrow.dots()
    arrow.set_dots([
        update(dot) if dot["is_focused"] else dot
        for dot in dots
    ])


def focus_adjacent_dot(delta):
    dots = arrow.dots()
    if not dots:
        return

    focused_indices = [
        index for index, dot in enumerate(dots)
        if dot["is_focused"]
    ]
    current = focused_indices[0] if focused_indices else (
        0 if delta > 0 else len(dots) - 1
    )
    target = (current + delta) % len(dots)
    arrow.set_dots([
        dict(dot, is_focused=index == target)
        for index, dot in enumerate(dots)
    ])


def move_horizontal(delta):
    text_length = len(arrow.text())
    update_focused_dots(lambda dot: {
        "cursor": max(0, min(text_length, dot["cursor"] + delta)),
        "selection": None,
        "is_focused": dot["is_focused"],
    })


def move_vertical(delta):
    text = arrow.text()
    lines = text.splitlines(True)
    if not lines:
        return
    offsets = []
    offset = 0
    for line in lines:
        offsets.append(offset)
        offset += len(line)

    def move(dot):
        cursor = dot["cursor"]
        before = text[:cursor]
        row = before.count("\n")
        column = len(before) - before.rfind("\n") - 1
        target_row = max(0, min(len(lines) - 1, row + delta))
        target_line = lines[target_row]
        target_column = min(column, max(0, len(target_line.rstrip("\n"))))
        target = offsets[target_row] + target_column
        return {
            "cursor": min(len(text), target),
            "selection": None,
            "is_focused": dot["is_focused"],
        }

    update_focused_dots(move)


def delete_dots():
    ranges = []
    for dot in arrow.dots():
        start, end = selected_range(dot)
        if start == end:
            end = min(len(arrow.text()), end + 1)
        ranges.append((start, end, ""))
    arrow.replace_ranges(ranges)


@arrow.command("insert-mode")
def insert_mode():
    arrow.enter_insert_mode()


@arrow.command("normal-mode")
def normal_mode():
    arrow.leave_insert_mode()


@arrow.command("toggle-file-explorer")
def toggle_file_explorer_command():
    arrow.toggle_file_explorer()


@arrow.command("move-left")
def move_left_command():
    move_horizontal(-1)


@arrow.command("move-down")
def move_down_command():
    move_vertical(1)


@arrow.command("move-up")
def move_up_command():
    move_vertical(-1)


@arrow.command("move-right")
def move_right_command():
    move_horizontal(1)


@arrow.command("focus-previous-dot")
def focus_previous_dot_command():
    focus_adjacent_dot(-1)


@arrow.command("focus-next-dot")
def focus_next_dot_command():
    focus_adjacent_dot(1)


@arrow.command("move-next-word")
def move_next_word_command():
    text = arrow.text()

    def move(dot):
        cursor = min(len(text), dot["cursor"] + 1)
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        while cursor < len(text) and not text[cursor].isspace():
            cursor += 1
        return {
            "cursor": cursor,
            "selection": None,
            "is_focused": dot["is_focused"],
        }

    update_focused_dots(move)


@arrow.command("move-previous-word")
def move_previous_word_command():
    text = arrow.text()

    def move(dot):
        cursor = max(0, dot["cursor"] - 1)
        while cursor > 0 and cursor < len(text) and text[cursor].isspace():
            cursor -= 1
        while cursor > 0 and cursor < len(text) and not text[cursor].isspace():
            cursor -= 1
        return {
            "cursor": cursor,
            "selection": None,
            "is_focused": dot["is_focused"],
        }

    update_focused_dots(move)


@arrow.command("select-line")
def select_line_command():
    text = arrow.text()

    def select(dot):
        cursor = dot["cursor"]
        start = text.rfind("\n", 0, cursor) + 1
        end = text.find("\n", cursor)
        if end == -1:
            end = len(text)
        return {
            "cursor": end,
            "selection": [start, end],
            "is_focused": dot["is_focused"],
        }

    update_focused_dots(select)


@arrow.command("append-mode")
def append_mode_command():
    text_length = len(arrow.text())
    update_focused_dots(lambda dot: {
        "cursor": min(text_length, dot["cursor"] + 1),
        "selection": None,
        "is_focused": dot["is_focused"],
    })
    arrow.enter_insert_mode()


@arrow.command("add-cursor-below")
def add_cursor_below_command():
    dots = arrow.dots()
    focused_dots = [dot for dot in dots if dot["is_focused"]]
    if not focused_dots:
        return
    text = arrow.text()
    lines = text.splitlines(True)
    if not lines:
        return
    for dot in focused_dots:
        before = text[:dot["cursor"]]
        row = before.count("\n")
        column = len(before) - before.rfind("\n") - 1
        if row + 1 >= len(lines):
            continue
        offset = sum(len(line) for line in lines[:row + 1])
        target = offset + min(column, len(lines[row + 1].rstrip("\n")))
        dots.append({"cursor": target, "selection": None, "is_focused": False})
    arrow.set_dots(dots)


@arrow.command("keep-primary-cursor")
def keep_primary_cursor_command():
    arrow.set_dots(arrow.dots()[:1])


@arrow.command("delete")
def delete_command():
    delete_dots()


@arrow.command("paste")
def paste_command():
    pass


@arrow.command("clear-selection")
def clear_selection_command():
    update_focused_dots(lambda dot: {
        "cursor": dot["cursor"],
        "selection": None,
        "is_focused": dot["is_focused"],
    })


@arrow.command("insert-newline")
def insert_newline_command():
    arrow.replace_ranges([
        (dot["cursor"], dot["cursor"], "\n")
        for dot in arrow.dots()
    ])


@arrow.command("backspace")
def backspace_command():
    ranges = []
    positions = []
    for dot in arrow.dots():
        start, end = selected_range(dot)
        if start == end:
            start = max(0, start - 1)
        ranges.append((start, end, ""))
        positions.append(start)
    arrow.replace_ranges(ranges)
    arrow.set_dots([{"cursor": position, "selection": None} for position in positions])


@arrow.command("close-file-explorer")
def close_file_explorer_command():
    arrow.close_file_explorer()


@arrow.command("explorer-up")
def explorer_up():
    arrow.explorer_selection_up()


@arrow.command("explorer-down")
def explorer_down():
    arrow.explorer_selection_down()


@arrow.command("explorer-parent")
def explorer_parent():
    arrow.explorer_parent_directory()


@arrow.command("explorer-open")
def explorer_open():
    arrow.explorer_open_selected()


@arrow.command("explorer-activate")
def explorer_activate():
    arrow.explorer_activate_selected()


NORMAL_BINDINGS = {
    "i": "insert-mode",
    "e": "toggle-file-explorer",
    "h": "move-left",
    "j": "move-down",
    "k": "move-up",
    "l": "move-right",
    "left": "move-left",
    "down": "move-down",
    "up": "move-up",
    "right": "move-right",
    "[": "focus-previous-dot",
    "]": "focus-next-dot",
    "w": "move-next-word",
    "b": "move-previous-word",
    "x": "select-line",
    "a": "append-mode",
    "c": "add-cursor-below",
    ",": "keep-primary-cursor",
    "d": "delete",
    "p": "paste",
    "escape": "clear-selection",
}

INSERT_BINDINGS = {
    "escape": "normal-mode",
    "enter": "insert-newline",
    "backspace": "backspace",
}

EXPLORER_BINDINGS = {
    "escape": "close-file-explorer",
    "up": "explorer-up",
    "down": "explorer-down",
    "k": "explorer-up",
    "j": "explorer-down",
    "left": "explorer-parent",
    "right": "explorer-open",
    "h": "explorer-parent",
    "l": "explorer-open",
    "enter": "explorer-activate",
}

for mode, bindings in (
    ("normal", NORMAL_BINDINGS),
    ("insert", INSERT_BINDINGS),
    ("explorer", EXPLORER_BINDINGS),
):
    for key, command_name in bindings.items():
        arrow.bind_key(key, command_name, mode)
