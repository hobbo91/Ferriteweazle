"""Writes an Extended CPC DSK (EDSK) image of one track, cylinder 0 head 0,
to the path given: nine sectors, R1 to R8 with R7 twice, of 512 bytes
where they hold data, as a uPD765 would report a disk holding each kind of
sector the disk view tells apart. gw lays the
track out from the image, faults and all, as its EDSK reader does.

  R1  good
  R2  deleted data, its CRC holding
  R3  data whose CRC fails
  R4  deleted data whose CRC fails
  R5  a header whose CRC fails, and so no data (gw writes none)
  R6  a header with no data after it
  R7  good, then R7 again, good, other data
  R8  good

Each sector's data counts up from a byte of its own, so no disk's data is
in it (see track.rs's every_recorded_report test)."""
import struct
import sys

GAP3 = 40  # bytes of 4E after each sector's data

# ST1's CRC error, which a header's or data's sets, and ST2's bits, as the
# uPD765 sets them and gw's EDSK reader reads them.
ST1_CRC = 0x20
ST2_DATA_CRC, ST2_DELETED, ST2_NO_DATA = 0x20, 0x40, 0x01

# R, ST1, ST2, the byte its data counts up from (None: no data stored).
SECTORS = [
    (1, 0, 0, 0x00),
    (2, 0, ST2_DELETED, 0x20),
    (3, ST1_CRC, ST2_DATA_CRC, 0x40),
    (4, ST1_CRC, ST2_DATA_CRC | ST2_DELETED, 0x60),
    (5, ST1_CRC, 0, None),
    (6, 0, ST2_NO_DATA, None),
    (7, 0, 0, 0x80),
    (7, 0, 0, 0xa0),
    (8, 0, 0, 0xc0),
]


def track():
    info = b'Track-Info\r\n' + bytes(4)
    # Cylinder, head, rate (1: double density), mode (2: MFM), N, sectors,
    # gap 3, filler.
    info += bytes([0, 0, 1, 2, 2, len(SECTORS), GAP3, 0xe5])
    data = b''
    for r, st1, st2, first in SECTORS:
        held = b'' if first is None else bytes((first + i) & 0xff for i in range(512))
        info += struct.pack('<6BH', 0, 0, r, 2, st1, st2, len(held))
        data += held
    info += bytes(256 - len(info))
    block = info + data
    return block + bytes(-len(block) % 256)


def disk():
    t = track()
    head = b'EXTENDED CPC DSK File\r\nDisk-Info\r\n' + b'ferriteweazle'.ljust(14)
    head += bytes([1, 1]) + bytes(2) + bytes([len(t) // 256])
    return head + bytes(256 - len(head)) + t


if __name__ == '__main__':
    with open(sys.argv[1], 'wb') as f:
        f.write(disk())
