Ferriteweazle for Linux
=======================

A desktop app for Greaseweazle. It runs on 64-bit x86 or ARM Linux with
glibc 2.17 or newer, under Wayland or X11, with Vulkan or OpenGL.

    ferriteweazle            the program
    greaseweazle/            Greaseweazle Tools and its Python
    lib/                     libraries some systems lack
    io.github.hobbo91.ferriteweazle.desktop
                             a menu entry, with ferriteweazle.png its icon

Keep greaseweazle/ and lib/ beside the program.

Running
-------
    ./ferriteweazle

The applications menu
---------------------
In this folder, run:

    mkdir -p ~/.local/share/applications
    sed -e "s|^Exec=.*|Exec=\"$PWD/ferriteweazle\"|" \
        -e "s|^Icon=.*|Icon=$PWD/ferriteweazle.png|" \
        io.github.hobbo91.ferriteweazle.desktop \
        >~/.local/share/applications/io.github.hobbo91.ferriteweazle.desktop

Run it again after moving the folder. Remove an entry made before 1.4.0:

    rm -f ~/.local/share/applications/ferriteweazle.desktop

Serial port access
------------------
Many distributions let only the dialout or uucp group open /dev/ttyACM*.
When gw is refused the port, Ferriteweazle offers to install gw's udev rule,
greaseweazle/49-greaseweazle.rules, through pkexec. The rule gives the
user logged in at the computer access to a Greaseweazle, and tells
ModemManager to leave it alone. To install it by hand:

    sudo cp greaseweazle/49-greaseweazle.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules && sudo udevadm trigger

Presets
-------
Example presets for common disks are in the repository:

    https://github.com/hobbo91/Ferriteweazle/tree/main/presets

Each page's Presets menu lists those in Documents/Ferriteweazle/Presets,
or the folder Settings > Paths names; Load... opens one from anywhere.

Ferriteweazle is MIT licensed; see LICENSE.txt. THIRD-PARTY-NOTICES.txt
holds the licences of the software it includes. Greaseweazle Tools is by
Keir Fraser and is in the public domain.
