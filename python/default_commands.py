import os
import arrow


@arrow.command("about-arrow")
def about_arrow():
    print("Arrow - a Python-extensible editor written in Rust")


@arrow.command("current-directory")
def current_directory():
    print(os.getcwd())


@arrow.command("list-directory")
def list_directory():
    for name in sorted(os.listdir(os.getcwd())):
        print(name)


@arrow.command("default-python-commands")
def default_python_commands():
    for name in sorted(arrow.commands):
        print(name)
