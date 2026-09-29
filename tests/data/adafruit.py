"""gw 1.23, through the bridge, against a stand-in for Adafruit's
Greaseweazle-compatible firmware: Adafruit_Floppy 0.6.1's
examples/greaseweazle/greaseweazle.ino, its replies ported command by command,
and its library's rp2040 flux writer (greasepack.h's greaseunpack,
arch_rp2.cpp's write_foreground). Prints what gw does with each.

    python adafruit.py BRIDGE SCRATCH_IMAGE_PATH
"""
import io, runpy, struct, sys
bridge = runpy.run_path(sys.argv[1])
IMAGE = sys.argv[2]  # gw's runs replace sys.argv
import serial
from greaseweazle import usb as USB

IDX = bytes([0xFF, 1, 1, 1, 1, 1])  # the index op the firmware sends


class Adafruit:
    """Adafruit_Floppy's examples/greaseweazle.ino, 0.6.1, as gw sees it over
    the port: its reply to each command, byte for byte. `model` 4 stands in
    for a Greaseweazle V4, to show what the bridge leaves alone."""

    def __init__(self, model=8):
        self.model, self.floppy, self.track, self.out = model, False, -1, b''
        self.timeout, self.seeks, self.flux, self.written, self.status = None, [], None, [], True

    baudrate = property(lambda self: 9600, lambda self, rate: None)
    in_waiting = property(lambda self: len(self.out))

    def reset_output_buffer(self): pass
    def reset_input_buffer(self): self.out = b''
    def close(self): pass
    def open(self): pass

    def read(self, n):
        got, self.out = self.out[:n], self.out[n:]
        assert len(got) == n or self.timeout is not None, 'gw would wait for ever'
        return got

    def reply(self, data):
        self.out += bytes(data)

    def write(self, data):
        if self.flux is not None:  # WRITEFLUX's stream, to its 0
            self.flux += data
            if 0 in self.flux:
                self.written.append(bytes(self.flux[:self.flux.index(0) + 1]))
                self.flux = None
                self.reply([0])
            return
        cmd, n = data[0], data[1]
        needs_floppy = {2, 3, 6, 7, 8, 12, 13, 15}
        if cmd in needs_floppy and not self.floppy:  # needfloppy:
            return self.reply([cmd, 1])
        if cmd == 0:  # GETINFO: firmware and bandwidth alone; no reply to others
            if data[2] == 0:
                info = bytes([cmd, 0, 1, 0, 1, 21]) + struct.pack('<I', 24000000)
                self.reply((info + bytes([self.model, 0, 0])).ljust(34, b'\0'))
            elif data[2] == 1:
                self.reply(bytes([cmd, 0]).ljust(34, b'\0'))
        elif cmd == 5:  # GETPARAMS: its 10 bytes of delays, whatever gw asks
            if data[2] == 0:
                self.reply([cmd, 0] + [1] * 10)
        elif cmd == 16:  # RESET
            self.reply([cmd, 0])
        elif cmd == 14:  # SETBUSTYPE: IBM, Shugart and both Apple II
            self.floppy = data[2] in (1, 2, 3, 4)
            self.reply([cmd, 0 if self.floppy else 1])
        elif cmd == 2:  # SEEK: goto_track steps no further than 79
            track = data[2] | (data[3] << 8 if n > 3 else 0)
            self.seeks.append(track)
            self.track = min(track, 79)
            self.reply([cmd, 0])
        elif cmd in (3, 6, 13):  # HEAD, MOTOR, DESELECT
            self.reply([cmd, 0])
        elif cmd == 12:  # SELECT: unit 0 alone
            self.reply([cmd, 0 if data[2] == 0 else 7])
        elif cmd == 7:  # READFLUX: 100 fluxes of 200 ticks a revolution
            ticks = struct.unpack('<I', data[2:6])[0]
            revs = struct.unpack('<H', data[6:8])[0]
            revs = 1 if ticks else max(revs - 1, 0)
            self.reply([cmd, 0])
            for rev in range(revs):
                self.reply(IDX + bytes([200] * 100))
                if rev == revs - 1 or ticks:
                    self.reply(IDX)
            self.reply([0])
            self.status = True
        elif cmd == 9:  # GETFLUXSTATUS
            self.reply([cmd, 0 if self.status else 10])
        elif cmd == 8:  # WRITEFLUX: not write protected
            self.reply([cmd, 0])
            self.flux = b''
            self.status = True
        elif cmd == 20:  # GETPIN: pin 26 alone, and a byte after either way
            at_zero = self.track == 0
            self.reply([cmd, 0, 0 if at_zero else 1] if data[2] == 26 else [cmd, 10, 0])
        elif cmd == 15:  # SETPIN: pin 2 alone
            self.reply([cmd, 0 if data[2] == 2 else 10])
        else:  # EraseFlux, SwitchFwMode, SetParams and the rest
            self.reply([cmd, 1])


def run(port, *args):
    """gw ARGS through the bridge, on `port`: its last line of output."""
    serial.Serial = lambda *a, **k: port
    out = io.TextIOWrapper(io.BytesIO(), write_through=True)
    saved = sys.stdout, sys.stderr
    sys.stderr = out
    try:
        bridge['gw'](list(args))
        waits = ''
    except AssertionError as e:
        waits = str(e)
    except SystemExit as e:
        waits = f'exit {e.code}'
    finally:
        sys.stdout, sys.stderr = saved
    lines = out.buffer.getvalue().decode().strip().splitlines()
    return waits or (lines[-1] if lines else '')


def written_after_start(stream):
    """Adafruit_Floppy's rp2040 write_foreground over greaseunpack: the flux
    values it hands the writer once it starts, from `stream` less its last 7
    bytes (greaseweazle.ino: fluxors - 7). Its TX FIFO takes 8 first."""
    buf, pos = stream[:len(stream) - 7], 0

    def unpack():
        nonlocal pos
        while True:
            if pos >= len(buf):
                return 0xFFFF
            left, data = len(buf) - pos, buf[pos]
            pos += 1
            need = 6 if data == 255 else 2 if data >= 250 else 1
            if left < need:
                pos = len(buf)
                return 0xFFFF
            if need == 1:
                return data
            if need == 2:
                pos += 1
                return (data - 250 + 1) * 250 + buf[pos - 1]
            pos += 1
            if buf[pos - 1] != 2:  # not FluxOp.Space: skipped
                pos += 4
                continue
            v = sum((buf[pos + i] & 254) << (7 * i) >> 1 for i in range(4))
            pos += 4
            return v

    fifo = [unpack() for _ in range(8)]
    after = 0
    while pos != len(buf):
        unpack()
        after += 1
    return fifo, after


fw = Adafruit()
print('info --bootloader:', run(fw, 'info', '--device', 'X', '--bootloader'))
print('pin set 2 H before any drive:', run(fw, 'pin', 'set', '--device', 'X', '2', 'H'))
print('seek 79:', run(fw, 'seek', '--device', 'X', '79'), fw.seeks)
print('seek 80:', run(fw, 'seek', '--device', 'X', '80'), fw.seeks)
print('seek 80 on a Greaseweazle:', run(Adafruit(4), 'seek', '--device', 'X', '80'))
print('rpm drive B:', run(fw, 'rpm', '--device', 'X', '--drive', 'B'))
print('rpm drive 0:', run(fw, 'rpm', '--device', 'X', '--drive', '0', '--nr', '2'))
print('pin get 26:', run(fw, 'pin', 'get', '--device', 'X', '26'))
print('pin get 25:', run(Adafruit(), 'pin', 'get', '--device', 'X', '25'))
print('pin set 2 H after a drive:', run(fw, 'pin', 'set', '--device', 'X', '2', 'H'))
print('pin set 4 H:', run(fw, 'pin', 'set', '--device', 'X', '4', 'H'))
print('delays:', run(Adafruit(), 'delays', '--device', 'X'))
print('reset:', run(Adafruit(), 'reset', '--device', 'X'))
print('erase:', run(Adafruit(), 'erase', '--device', 'X', '--tracks', 'c=0:h=0'))
hf = Adafruit()
print('erase --hfreq:', run(hf, 'erase', '--device', 'X', '--tracks', 'c=0:h=0', '--hfreq'))
fifo, after = written_after_start(hf.written[0])
print('hfreq flux written after the writer starts:', after)
unit = object.__new__(USB.Unit)
unit.sample_freq = 24000000
fifo, after = written_after_start(bytes(unit._encode_flux([96] * 1000)))
print('ordinary flux written after the writer starts:', after)
print('read --densel H:', run(Adafruit(), 'read', '--device', 'X', '--densel', 'H', '--format', 'ibm.1440', IMAGE))
blank = IMAGE + '.img'
with open(blank, 'wb') as f:
    f.write(bytes(1474560))
print('write --pre-erase:', run(Adafruit(), 'write', '--device', 'X', '--pre-erase', '--format', 'ibm.1440', '--tracks', 'c=0:h=0', blank))
print('write:', run(Adafruit(), 'write', '--device', 'X', '--no-verify', '--format', 'ibm.1440', '--tracks', 'c=0:h=0', blank))

# Detect's search for a track that tells formats apart, stopped at cylinder
# 79 as it is for model 8: two formats alike to 79 and apart from 80.
divergence = bridge['divergence']
divergence.__globals__['layout'] = lambda disk, key, cache: key[0] >= 80 and disk == 'b'


class Disk(str):
    cyls = 84


pair, read = [Disk('a'), Disk('b')], {(0, 0): None}
print('detect looks as far as cylinder 79:', divergence(pair, read, 1, {}, 79), divergence(pair, read, 1, {}, 83))
