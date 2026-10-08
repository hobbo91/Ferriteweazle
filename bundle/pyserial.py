"""Gives the pyserial under LIB, a bundle's lib/pythonX.Y, its fix for glibc
2.42: pyserial's commit 70d1886, which no pyserial release has yet.

gw clears its link to the Greaseweazle by setting 10000 baud, a rate with no
Bxxx code. pyserial 3.5 hands such a rate to tcsetattr() as BOTHER, then sets
it with TCSETS2. Since glibc 2.42, a program built for an older glibc, as the
bundle's Python is, gets the cfsetispeed() and cfsetospeed() of the old codes,
which refuse BOTHER, and tcsetattr() fails: termios.error (22, 'Invalid
argument'), before gw has said a word to the Greaseweazle. The fix hands
tcsetattr() B38400 instead; TCSETS2 still sets the rate. A pyserial with the
fix is left as it is."""

import pathlib
import sys

OLD = """\
                # See if BOTHER is defined for this platform; if it is, use
                # this for a speed not defined in the baudrate constants list.
                try:
                    ispeed = ospeed = BOTHER
                except NameError:
                    # may need custom baud rate, it isn't in our list.
                    ispeed = ospeed = getattr(termios, 'B38400')
"""
FIXED = """\
                # Use safe placeholder for tcsetattr(), try to set special baudrate later
                ispeed = ospeed = termios.B38400
"""

path = pathlib.Path(sys.argv[1], "site-packages", "serial", "serialposix.py")
text = path.read_text()
if FIXED not in text:
    if text.count(OLD) != 1:
        sys.exit(f"bundle: {path} is not pyserial 3.5's; give it pyserial's 70d1886 by hand")
    path.write_text(text.replace(OLD, FIXED))
