import arrow


@arrow.command("execute-command")
def execute_command(command):
    arrow.run_command(command)


@arrow.command("toggle-vertico")
def toggle_vertico_command():
    items = [(name, name) for name in sorted(arrow.commands)]
    arrow.show_vertico("Commands", items, "execute-command", "M-x")


@arrow.command("close-vertico")
def close_vertico_command():
    arrow.close_vertico()


@arrow.command("vertico-up")
def vertico_up_command():
    arrow.vertico_selection_up()


@arrow.command("vertico-down")
def vertico_down_command():
    arrow.vertico_selection_down()


@arrow.command("vertico-backspace")
def vertico_backspace_command():
    arrow.vertico_backspace()


@arrow.command("execute-vertico")
def execute_vertico_command():
    arrow.execute_vertico()


VERTICO_BINDINGS = {
    "escape": "close-vertico",
    "up": "vertico-up",
    "down": "vertico-down",
    "tab": "vertico-down",
    "backspace": "vertico-backspace",
    "enter": "execute-vertico",
}

for key, command_name in VERTICO_BINDINGS.items():
    arrow.bind_key(key, command_name, "vertico")

arrow.bind_key("space", "toggle-vertico", "normal")
arrow.bind_key("tab", "toggle-vertico", "normal")
