Ferriteweazle for Linux
=======================

A desktop app for Greaseweazle. It runs on 64-bit x86 or ARM Linux with
glibc 2.17 or newer, under Wayland or X11, with Vulkan or OpenGL.

    ferriteweazle            the program
    ferriteweazle-data/      its Python and gw; keep it beside the program
    ferriteweazle.desktop    a menu entry, with ferriteweazle.png its icon

Running
-------
    ./ferriteweazle

The applications menu
---------------------
In this folder, run:

    mkdir -p ~/.local/share/applications
    sed -e "s|^Exec=.*|Exec=\"$PWD/ferriteweazle\"|" \
        -e "s|^Icon=.*|Icon=$PWD/ferriteweazle.png|" \
        ferriteweazle.desktop >~/.local/share/applications/ferriteweazle.desktop

Run it again after moving the folder.

Serial port access
------------------
Many distributions let only the dialout or uucp group open /dev/ttyACM*.
When gw is refused the port, Ferriteweazle offers to install gw's udev rule,
ferriteweazle-data/49-greaseweazle.rules, through pkexec. The rule gives the
user logged in at the computer access to a Greaseweazle, and tells
ModemManager to leave it alone. To install it by hand:

    sudo cp ferriteweazle-data/49-greaseweazle.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules && sudo udevadm trigger

Ferriteweazle is MIT licensed; see LICENSE.txt. THIRD-PARTY-NOTICES.txt
holds the licences of the software it includes. Greaseweazle is by Keir
Fraser and is in the public domain.
