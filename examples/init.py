import os
import arrow


@arrow.command("hello-world")
def hello_world():
    print("Hello from Arrow!")


@arrow.command("show-working-directory")
def show_working_directory():
    print(os.getcwd())


@arrow.command("reload-config")
def reload_config():
    print("Restart Arrow to reload its Python configuration.")


@arrow.hook("startup")
def startup():
    print("Example Arrow configuration loaded.")
