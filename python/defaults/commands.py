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


@arrow.command("open-readme")
def open_readme():
    arrow.open_file("README.md")


@arrow.command("write-output-file")
def write_output_file():
    arrow.write_file("arrow-output.txt", "Written by Arrow's Python scripting system.\n")
    print("Wrote arrow-output.txt")
