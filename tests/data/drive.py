"""gw 1.23's write through the bridge, to a stand-in drive that turns at
exactly 300 rpm, keeps each track as gw writes it from the index, and reads
it back from wherever it is when gw verifies. Makes IMAGE first, of random
bytes in FORMAT; with FORMAT empty, writes IMAGE, a flux image, as it is.
Prints gw's output, with the bridge's reports, and STAND_IN once the
stand-in is in use. gw is given a port that does not exist, so that it could
not reach a real drive even if it got past the stand-in. With FAIL_AT set to
a track, such as 3.0, writing it fails, as a write-protected disk's does.

    python drive.py BRIDGE FORMAT IMAGE [gw write options...]
"""
import contextlib, io, os, random, runpy, sys
bridge = runpy.run_path(sys.argv[1])
fmt, image, options = sys.argv[2], sys.argv[3], sys.argv[4:]
from greaseweazle import usb
from greaseweazle.codec import codec
from greaseweazle.flux import Flux
from greaseweazle.tools import util

if fmt:
    with contextlib.redirect_stdout(io.StringIO()):
        d = codec.get_diskdef(fmt)
    size = sum(len(t.get_img_track()) for c in range(d.cyls) for h in range(d.heads)
               if (t := d.mk_track(c, h)))
    rand = random.Random(fmt)
    with open(image, 'wb') as f:
        f.write(rand.getrandbits(8 * size).to_bytes(size, 'little'))


STAND_IN = 'drive.py: the stand-in drive'
NO_SUCH_PORT = '/dev/ferriteweazle-no-such-port'


class Drive:
    """A Greaseweazle's usb.Unit as gw's write uses it, over a perfect drive."""
    sample_freq = 72_000_000

    def __init__(self):
        print(STAND_IN, file=sys.stderr, flush=True)
        self.period = self.sample_freq * 60 / 300
        self.tracks, self.at, self.rand = {}, (0, 0), random.Random(1)

    def seek(self, cyl, head): self.at = (cyl, head)
    def set_bus_type(self, bus): pass
    def drive_select(self, unit): pass
    def drive_motor(self, unit, on): pass
    def drive_deselect(self): pass
    def reset(self): pass
    def get_pin(self, pin): return True
    def set_pin(self, pin, level): pass
    def erase_track(self, ticks): self.tracks.pop(self.at, None)

    def write_track(self, flux_list, cue_at_index=True, terminate_at_index=True,
                    hard_sector_ticks=0):
        assert cue_at_index and not hard_sector_ticks
        if os.environ.get('FAIL_AT') == '%d.%d' % self.at:
            # As a Greaseweazle answers a write to a write-protected disk.
            raise usb.CmdError(bytes([usb.Cmd.WriteFlux, 8]), usb.Ack.Wrprot)
        times, t = [], 0.0
        for f in flux_list:
            t += f
            times.append(t)
        p = self.period
        if terminate_at_index:
            times = [x for x in times if x < p]
            t = min(t, p)
        # Past one revolution, the write runs on over its own start.
        kept = [x - p for x in times if x >= p] + [x for x in times if t - p <= x < p]
        self.tracks[self.at] = sorted(kept)

    def read_track(self, revs, ticks=0):
        """From a random place: to the first index, then `revs` revolutions,
        or for `ticks`, as the firmware stops at whichever comes first."""
        track, p = self.tracks.get(self.at, []), self.period
        start = self.rand.uniform(0, p)
        length = (p - start) + revs * p
        if ticks:
            length = min(length, ticks)
        times = [k * p + x for k in range(revs + 2) for x in track]
        flux, last = [], start
        for x in (x for x in times if start < x <= start + length):
            flux.append(x - last)
            last = x
        index = [p - start] + [p] * revs
        while index and sum(index) > length + 1e-6:
            index.pop()
        return Flux(index, flux, self.sample_freq, index_cued=False)


util.usb_open = lambda *a, **k: Drive()
bridge['gw'](['write', f'--device={NO_SUCH_PORT}', *([f'--format={fmt}'] if fmt else []),
              *options, image])
