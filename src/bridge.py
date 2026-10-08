"""Ferriteweazle's link to gw, through gw's own modules, patched only by
steady_handshake, bundled_caps, adafruit_seeks, read_passes and the report_*
hooks.

Modes: serve (one JSON request per stdin line, one reply per stdout line),
run ARGS (`gw ARGS`; stdin takes 'answer TEXT', anything else stops it),
detect ARGS, latest [REPO], update TAG BUNDLED DIR and fetch TAG NAME DIR."""
import argparse, bisect, builtins, collections, contextlib, copy, functools, importlib, io, itertools, json, math, os, queue, re, signal, struct, sys, threading, typing, weakref, _thread

# Must match job.rs.
ASK = '@ferriteweazle ask '
RESULT = '@ferriteweazle result '
TRACK = '@ferriteweazle track '
IMAGE = '@ferriteweazle image '
VERIFY = '@ferriteweazle verify '

# Detect's word where no format reads every sector: of a disk, gw reads it
# as raw flux with no format; of an image, gw takes its tracks as they are.
UNDETECTED_DISK = ('No format Greaseweazle Tools knows reads this disk in full. '
                   'Set Disk format to None to read as raw flux (.scp).')
UNDETECTED_IMAGE = ('No format Greaseweazle Tools knows reads this image in full. '
                    'Set Disk format to None to use its tracks as they are.')

# The equal parts of a revolution a track's report counts its flux in: a
# quarter of a degree each.
BINS = 1440

# A track's report counts its flux's intervals up to INTERVAL_TOP long, in
# bins of whole sample ticks as near INTERVAL_BIN wide as the ticks allow.
INTERVAL_TOP = 20e-6
INTERVAL_BIN = 50e-9

# Tests point these at a server of their own.
GITHUB = os.environ.get('FERRITEWEAZLE_GITHUB', 'https://github.com')
GITHUB_API = os.environ.get('FERRITEWEAZLE_GITHUB_API', 'https://api.github.com')
GW_REPO = 'keirf/greaseweazle'
APP_REPO = 'hobbo91/Ferriteweazle'  # must match update.rs's APP_REPO


class Captured(Exception):
    pass


def parser_for(mod, argv):
    """Returns the parser `gw ARGV` builds, or None and whatever it printed."""
    def capture(self, *a, **k):
        raise Captured(self)
    real, argparse.ArgumentParser.parse_args = argparse.ArgumentParser.parse_args, capture
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out):
            mod.main(['gw'] + argv)
    except Captured as c:
        return c.args[0], ''
    except SystemExit:
        pass
    finally:
        argparse.ArgumentParser.parse_args = real
    return None, out.getvalue()


def type_name(t):
    if t is None:
        return None
    q = getattr(t, '__qualname__', None) or type(t).__qualname__
    return q.split('.<locals>')[0]


def arg(a, groups, prog):
    h = (a.help or '').replace('%no_default', '')
    with contextlib.suppress(KeyError, TypeError, ValueError):
        h = h % dict(vars(a), prog=prog)
    d = {
        'flags': a.option_strings,
        'dest': a.dest,
        'switch': a.nargs == 0 or None,
        'type': type_name(a.type),
        'default': str(a.default) if type(a.default) in (str, int, float) else None,
        'choices': [str(c) for c in a.choices] if a.choices else None,
        'required': a.required or None,
        'group': groups.get(id(a)),
        # The kind of value, such as TSPEC, whose help is in the schema's notes.
        'metavar': a.metavar if isinstance(a.metavar, str) else None,
        'help': h.strip() or None,
    }
    return {k: v for k, v in d.items() if v is not None}


def command(name, mod, p):
    groups = {id(a): i for i, g in enumerate(p._mutually_exclusive_groups)
              for a in g._group_actions}
    skip = (argparse._HelpAction, argparse._VersionAction)
    # gw gives pin get and pin set one description, which is set's.
    about = 'Read the level of a user-modifiable interface pin.' if name == 'pin get' \
        else mod.description
    return {'name': name, 'about': about,
            'args': [arg(a, groups, p.prog) for a in p._actions if not isinstance(a, skip)]}


def commands(name):
    mod = importlib.import_module('greaseweazle.tools.' + name)
    p, usage = parser_for(mod, [name])
    if p:
        return [command(name, mod, p)]
    # A command with subcommands prints e.g. "usage: gw pin get|set".
    m = re.search(rf'{name} (\w+(?:\|\w+)+)', usage)
    subs = [f'{name} {s}' for s in m.group(1).split('|')] if m else []
    return [command(s, mod, p) for s in subs if (p := parser(s))]


def opts_class(cls):
    """The options class behind `file.ext::opt=val`: the one its module defines."""
    from greaseweazle.image.image import ImageOpts
    mod = sys.modules[cls.__module__]
    own = [v for v in vars(mod).values() if isinstance(v, type)
           and issubclass(v, ImageOpts) and v.__module__ == mod.__name__]
    return own[0] if len(own) == 1 else type(cls.opts)


def settings(o, names):
    """File options with their defaults and named values, if any. A bool
    default marks a flag, set by `::name` since gw takes any value as true."""
    try:
        inst = o()
    except Exception:
        inst = None
    out = []
    for n in names:
        d = getattr(inst, n, None)
        opt = {'name': n, 'default': d if type(d) in (bool, int, float, str) else None}
        if choices := named(o, n):
            opt['choices'] = choices
            # The default by its name, as gw lists it: other-320k, not 128.
            opt['default'] = next((c for c in choices if set_to(o, n, c) == d), opt['default'])
        out.append(opt)
    return out


def named(o, n):
    """The names gw lists when it refuses a value of option n, if it lists any."""
    try:
        setattr(o(), n, '\x01')
    except Exception as e:
        lines = str(e).split('\n')
        at = next((i for i, line in enumerate(lines) if line.startswith('Valid')), None)
        if at is not None:
            return ' '.join(lines[at + 1:]).split()
    return []


def set_to(o, n, value):
    """What option n holds once set to value."""
    inst = o()
    setattr(inst, n, value)
    return getattr(inst, n)


def check_opt(ext, name, value):
    """gw's objection to a value of a file option, from its own setter, or None."""
    from greaseweazle.tools import util
    try:
        set_to(opts_class(util.get_image_class('x' + ext)), name, value)
    except Exception as e:
        return str(e).strip().split('\n')[0]
    return None


def images():
    from greaseweazle.tools import util
    from greaseweazle.image.image import Image
    from greaseweazle.image.img import IMG
    out = {}
    for ext, spec in util.image_types.items():
        cls = util.get_image_class('x' + ext)
        o = opts_class(cls)
        sectors = issubclass(cls, IMG)
        own_open = cls.from_file.__func__ is not Image.from_file.__func__
        out[ext] = {'name': spec if isinstance(spec, str) else spec[0],
                    'writable': not cls.read_only,
                    'default_format': cls.default_format,
                    'finds_format': finds_format(cls),
                    # Tracks as they lie on the disk, flux or decoded, not sectors.
                    'tracks': not sectors,
                    # A sector image gw opens only with a format, as it does an .img.
                    'needs_format': sectors and not cls.default_format and not own_open,
                    'read_opts': settings(o, o.a_settings + o.r_settings),
                    'write_opts': settings(o, o.a_settings + o.w_settings)}
    return out


def finds_format(cls):
    """Whether gw finds the disk format in the file, as it does an .nsi's from its size."""
    return callable(getattr(cls, 'format_from_file', None))


def image_format(path):
    """The format gw write and convert take from an image when none is chosen:
    its type's own, or one found in the file. None if one must be chosen."""
    from greaseweazle.tools import util
    cls = util.get_image_class(path)
    if cls.default_format or not finds_format(cls):
        return cls.default_format
    if not os.path.isfile(path):
        raise ValueError('There is no such file.')
    with quiet():
        cls.from_file(path, None, {})  # fails where gw would
    return cls.format_from_file(path)


def formats(diskdefs=None):
    """Format names with their numbers in numeric order: ibm.360 before ibm.1200."""
    from greaseweazle.codec import codec
    return sorted(codec.get_all_formats('', codec.DiskDef_File(diskdefs)),
                  key=lambda name: [int(p) if p.isdigit() else p for p in re.split(r'(\d+)', name)])


def diskdefs(path):
    """The formats a disk definitions file adds that gw can use, those it
    cannot, and gw's errors, each once. gw reads the file only as far as the
    format asked for, so a broken definition spoils only those after it."""
    from greaseweazle.codec import codec
    path = os.path.expanduser(path)  # as gw does
    if not os.path.isfile(path):
        raise ValueError('There is no such file.')
    names = formats(path)
    usable, failed, errors = [], [], []
    for name in names:
        try:
            with quiet():
                codec.get_diskdef(name, path)
            usable.append(name)
        except Exception as e:
            failed.append(name)
            if (error := str(e) or type(e).__name__) not in errors:
                errors.append(error)
    if not names:
        errors.append('Defines no usable disk.')
    return {'formats': usable, 'failed': failed, 'errors': errors}


def format_info(name, diskdefs=None):
    from greaseweazle.codec import codec
    d = codec.get_diskdef(name, diskdefs)
    if d is None:
        raise ValueError(f'unknown format: {name}')
    tracks = [t for c in range(d.cyls) for h in range(d.heads) if (t := d.mk_track(c, h))]
    info = {'cyls': d.cyls, 'heads': d.heads}
    if tracks:
        # Every encoding on the disk; a scan's "IBM Empty" tracks count as IBM.
        names = dict.fromkeys(re.sub(r'\s*(\(.*|Empty)$', '', t.summary_string()) for t in tracks)
        info['encoding'] = ' and '.join(names)
        with contextlib.suppress(Exception):  # a gw before 1.14 has no default_revs
            info['revs'] = d.default_revs
        if most := max(t.nsec for t in tracks):
            info['sectors'] = [min(t.nsec for t in tracks), most]
        with contextlib.suppress(Exception):
            if size := sum(len(t.get_img_track()) for t in tracks):
                info['bytes'] = size
    return info


def fits(ext, name, diskdefs=None):
    """gw's objection to an image of type `ext` in format `name`, or None, from
    one made in memory as a read makes it, then read back if it holds sectors."""
    from greaseweazle.codec import codec
    from greaseweazle.image.img import IMG
    from greaseweazle.tools import util
    d = codec.get_diskdef(name, diskdefs)
    if d is None:
        raise ValueError(f'unknown format: {name}')
    cls = util.get_image_class('x' + ext)
    try:
        with quiet():
            image = cls.to_file('x' + ext, d, False, {})
            for c in range(d.cyls):
                for h in range(d.heads):
                    if (t := d.mk_track(c, h)) is not None:
                        image.emit_track(c, h, t)
            data = image.get_image()
            if issubclass(cls, IMG):
                cls('x' + ext, d).from_bytes(data)
    except Exception as e:
        # Some fail on an assert with no message.
        return str(e).strip().split('\n')[0] or 'gw cannot make this image type of the format.'
    return None if data else 'The image would be empty.'


def ports():
    """Every serial port, best first, scored by gw's guess at a Greaseweazle:
    0 for ports that are not one. On Linux, denied if this account may not
    open it: `gw info` then says only that it found no device."""
    from greaseweazle.tools import util
    found = sorted(((util.score_port(p), p) for p in util.comports()),
                   key=lambda x: (-x[0], x[1].device))
    linux = sys.platform.startswith('linux')
    return [{'device': p.device,
             'name': next((n for n in (p.product, p.description) if n and n != 'n/a'), None),
             'score': s,
             'denied': linux and not os.access(p.device, os.R_OK | os.W_OK)}
            for s, p in found]


@functools.lru_cache(None)
def parser(command):
    argv = command.split()
    return parser_for(importlib.import_module('greaseweazle.tools.' + argv[0]), argv)[0]


def check(command, dest, value):
    """Checks a value with gw's own parser for that argument."""
    a = next(a for a in parser(command)._actions if a.dest == dest)
    if a.type:
        try:
            a.type(value)
        except Exception as e:
            return str(e) or f'invalid {type_name(a.type)} value: {value!r}'
    return None


def schema():
    from greaseweazle import cli, __version__
    from greaseweazle.tools import util
    notes = {v.split(':')[0]: v for k, v in vars(util).items()
             if k.endswith('_desc') and isinstance(v, str)}
    return {'version': __version__,
            'commands': [c for a in cli.actions for c in commands(a)],
            'formats': formats(),
            'images': images(),
            'notes': notes}


def serve():
    ops = {'schema': schema, 'formats': formats, 'format': format_info,
           'diskdefs': diskdefs, 'image_format': image_format, 'ports': ports, 'check': check,
           'fits': fits, 'check_opt': check_opt, 'image': image}
    out, sys.stdout = sys.stdout, sys.stderr  # stray prints must not corrupt replies
    for line in sys.stdin:
        req = json.loads(line)
        try:
            reply = {'ok': ops[req.pop('op')](**req)}
        except Exception as e:
            reply = {'error': str(e) or type(e).__name__}
        print(json.dumps(reply), file=out, flush=True)


def stop():
    if hasattr(signal, 'pthread_kill'):  # interrupts a blocking read too
        signal.pthread_kill(threading.main_thread().ident, signal.SIGINT)
    else:
        _thread.interrupt_main()


def detect(argv):
    """Finds the formats that read the disk in the drive, or a flux image, in
    full, and prints them best first after RESULT. It reads the drive as gw
    read does and a file as gw convert does, each with the options it takes."""
    # convert imports gw's codecs, which gw 1.23's track image modules
    # import in a circle.
    from greaseweazle.tools import convert, util
    p = argparse.ArgumentParser(prog='detect')
    p.add_argument('--device')
    p.add_argument('--drive', default='A')
    p.add_argument('--diskdefs')
    # gw's default. Only head offsets and hswap count: probe finds the step.
    p.add_argument('--tracks', type=util.TrackSet, default='c=0-81:h=0-1')
    p.add_argument('--densel', type=util.level)
    p.add_argument('--gen-tg43', action='store_true')
    p.add_argument('--fake-index', type=util.period)
    p.add_argument('--hard-sectors', action='store_true')
    p.add_argument('--reverse', action='store_true')
    p.add_argument('--adjust-speed', type=util.period)
    p.add_argument('file', nargs='?')
    a = p.parse_args(argv)
    a.tracks.step, a.fmt_cls = 1, None
    found, seen = [], {}
    guarded(scaling)
    if a.file:
        image = util.get_image_class(a.file).from_file(a.file, None, {})

        def read(c, h):
            return convert.process_input_track(a, convert.TrackIdentity(a.tracks, c, h), image)

        found.append(probe(reporting(read, a, seen), a.diskdefs))
    else:
        usb = util.usb_open(a.device)
        pin2 = a.densel is not None or a.gen_tg43
        level = usb.get_pin(2) if pin2 else None
        try:
            if a.densel is not None:
                usb.set_pin(2, a.densel)
            last = ADAFRUIT_LAST if usb.hw_model == ADAFRUIT_MODEL else 83
            read = reporting(drive_reader(usb, a), a, seen)
            util.with_drive_selected(lambda: found.append(probe(read, a.diskdefs, last)),
                                     usb, util.Drive()(a.drive))
        finally:
            if pin2:
                usb.set_pin(2, level)
    ranked, step, disks = found[0]
    whole = [m.name for m in ranked if m.full]
    # Each track read, reported again as the format found decodes it; places
    # are noted only now, after ranking.
    if whole and round_from_index(a):
        guarded(report_places)
        for (c, h), track in seen.items():
            report(c, h, track, decoded_as(disks[whole[0]], (c // step, h), track))
    for m in ranked[:6]:
        fit = f', {m.misfit:.1%} from its layout' if m.full else ', not all'
        print(f'{m.name}: {m.found} of {m.expected} sectors{fit}')
    if step > 1:
        print('This is a 40-track disk in an 80-track drive: it needs Step 2.')
    print(RESULT + json.dumps({'formats': whole, 'step': step}), flush=True)
    if not whole:
        print('** FATAL ERROR:\n' + (UNDETECTED_IMAGE if a.file else UNDETECTED_DISK))
        return 1
    print(f'Format {whole[0]}')
    return 0


def reporting(read, a, seen):
    """`read(c, h)` for probe, made to report each track it reads, as gw's
    line on it names it, and keep it in `seen` for its report again once
    the format is found. None of a track an image lacks, nor where tracks do
    not run round from the index."""
    def read_reporting(c, h):
        track = read(c, h)
        if track is not None:
            seen[c, h] = track
            if round_from_index(a):
                report(c, h, track, None)
        return track
    return read_reporting


def decoded_as(disk, key, track):
    """`track` decoded as format `disk` has its track `key`, as probe
    decodes it to weigh the format; None if that fails."""
    with quiet(), contextlib.suppress(Exception):
        t = disk.mk_track(*key)
        t.decode_flux(track)
        return t
    return None


def decoded_like(made, flux):
    """`flux` decoded as an IBM-style track of the mode, rate and revolution
    of `made`, with no layout of its own to fill: what is there, as gw's
    decoder finds it. None for other codecs, which have no such track."""
    from greaseweazle.codec.ibm import ibm
    if not isinstance(made, ibm.IBMTrack):
        return None
    with quiet(), contextlib.suppress(Exception):
        t = ibm.IBMTrack(made.cyl, made.head, made.mode)
        t.clock, t.time_per_rev = made.clock, made.time_per_rev
        t.decode_flux(flux)
        return t
    return None


def drive_reader(usb, a):
    """read(c, h) for probe: a track of the drive, as gw read reads one."""
    from greaseweazle.tools import convert, read
    a.raw, a.revs, a.ticks, a.drive_ticks_per_rev = False, 2, 0, None
    # As read_to_image: the fake index's period, or the drive's sector holes.
    if a.fake_index is not None:
        a.drive_ticks_per_rev = a.fake_index * usb.sample_freq
    elif a.hard_sectors:
        flux = usb.read_track(revs=0, ticks=int(usb.sample_freq / 2))
        flux.identify_hard_sectors()
        a.drive_ticks_per_rev = flux.ticks_per_rev
        a.hard_sectors = len(flux.sector_list[-1])
        print(f'Drive reports {a.hard_sectors} hard sectors')
        a.revs = (a.hard_sectors + 1) * (a.revs + 1)
    return lambda c, h: read.read_with_retry(usb, a, convert.TrackIdentity(a.tracks, c, h))[0]


# Layouts whose sectors sit within 1% of a revolution of the best fit
# count as tied: real drives and formatters vary that much.
FIT_TOLERANCE = 0.01


class Match(typing.NamedTuple):
    full: bool
    found: int
    expected: int
    misfit: float
    name: str


def probe(read, diskdefs, last=83):
    """Formats ranked by the tracks `read(cyl, head)` returns, the head step
    the disk needs, and the formats tried, by name. Decoding checks only
    sector IDs, sizes and data rate, leaving 22 groups of formats alike in gw
    1.23, so rank() weighs where the sectors sit, and tracks that tell the
    leaders apart are read."""
    from greaseweazle.codec import codec
    disks = {}
    # gw's own formats, then a definitions file's, which win a shared name.
    for source in [None, diskdefs] if diskdefs else [None]:
        for name in codec.get_all_formats('', codec.DiskDef_File(source)):
            if name.endswith('.scan'):  # scans read any layout, so prove nothing
                continue
            with contextlib.suppress(Exception):
                disks[name] = codec.get_diskdef(name, source)
    print('Trying every format Greaseweazle Tools knows...')
    tracks = {k: read(*k) for k in [(0, 0), (0, 1)]}
    decoded = {}
    ranked = rank(disks, tracks, decoded)
    if not any(m.full for m in ranked):
        return ranked, 1, disks
    # A 40-track disk in an 80-track drive has cylinder 1 at physical 2.
    second = read(2, 0)
    step = stepping(disks[ranked[0].name], second)
    tracks[2 // step, 0] = second
    ranked = rank({m.name: disks[m.name] for m in ranked if m.full}, tracks, decoded)
    layouts = {}
    for _ in range(4):
        leaders = [m for m in ranked if m.full]
        if not leaders:
            break
        # Only formats tied with the best need another track.
        best = min(m.misfit for m in leaders)
        close = [disks[m.name] for m in leaders if m.misfit <= best + FIT_TOLERANCE]
        key = divergence(close, tracks, step, layouts, last)
        if key is None:
            break
        tracks[key] = read(key[0] * step, key[1])
        ranked = rank({m.name: disks[m.name] for m in leaders}, tracks, decoded)
    return filesystem(ranked, disks, tracks, read, step), step, disks


def stepping(disk, track):
    """2 if physical cylinder 2 reads as this format's cylinder 1, not 2."""
    def at(c):
        tdef = disk.track_map.get((c, 0))
        return decode(tdef, (c, 0), track)[0] if tdef else 0
    return 2 if at(1) > at(2) else 1


def divergence(disks, tracks, step, layouts, last):
    """The first unread track to physical cylinder `last` that tells apart
    two of these formats that every track read so far leaves alike, or None."""
    def alike(a, b, key):
        return layout(a, key, layouts) == layout(b, key, layouts)
    pairs = [(a, b) for i, a in enumerate(disks) for b in disks[i + 1:]
             if all(alike(a, b, k) for k in tracks)]
    if not pairs:
        return None
    for c in range(max(d.cyls for d in disks)):
        if c * step > last:
            break
        for h in range(2):
            if (c, h) not in tracks and any(not alike(a, b, (c, h)) for a, b in pairs):
                return c, h
    return None


def layout(disk, key, cache):
    """What a format writes on a track, as far as the disk shows it: each
    sector's ID, size and place, the index mark and the data rate for IBM
    tracks; the codec and sector count for others."""
    tdef = disk.track_map.get(key)
    if tdef is None:
        return None
    sig = signature(key, tdef)
    if sig not in cache:
        try:
            with quiet():
                t = tdef.mk_track(*key)
            if is_ibm(t):
                per_rev = t.time_per_rev / t.clock
                where = tuple((*sector_id(x), round(x.start / per_rev, 3)) for x in t.sectors)
                cache[sig] = ('ibm', str(t.mode), round(1 / t.clock), bool(t.iams), where)
            else:
                cache[sig] = (type(t).__name__, t.nsec)
        except Exception:
            cache[sig] = sig
    return cache[sig]


def signature(key, tdef):
    """Formats that define a track alike write and decode it alike."""
    return key, type(tdef).__name__, repr(sorted(vars(tdef).items()))


def sector_id(s):
    return s.idam.c, s.idam.h, s.idam.r, s.idam.n


def is_ibm(track):
    """An IBM-style track, whose sectors gw places and finds by position."""
    return hasattr(getattr(track, 'raw', None), 'sectors') and hasattr(track, 'sectors')


def filesystem(ranked, disks, tracks, read, step):
    """Apple II formats write the same disk and differ only in the image's
    sector order, so the filesystem chooses: a ProDOS volume directory in
    block 2, or a DOS 3.3 table of contents on track 17."""
    whole = [m.name for m in ranked if m.full]
    if len(whole) < 2 or not all(n.startswith('apple2.') for n in whole[:2]):
        return ranked

    def sectors(name, key, track):
        with quiet(), contextlib.suppress(Exception):
            t = disks[name].track_map[key].mk_track(*key)
            t.decode_flux(track)
            return bytes(t.get_img_track())
        return b''

    choice = 'apple2.nofs.140'
    block2 = sectors('apple2.prodos.140', (0, 0), tracks[0, 0])[1024:1536]
    if len(block2) > 4 and block2[:2] == b'\0\0' and block2[4] >> 4 == 0xF:
        choice = 'apple2.prodos.140'
    elif 'apple2.appledos.140' in whole:
        vtoc = sectors('apple2.appledos.140', (17, 0), read(17 * step, 0))[:256]
        if len(vtoc) > 0x35 and (vtoc[1], vtoc[3], vtoc[0x34], vtoc[0x35]) == (17, 3, 35, 16):
            choice = 'apple2.appledos.140'
    print(f'Apple II filesystem: {choice}')
    return sorted(ranked, key=lambda m: (not m.full, m.name != choice))


def rank(disks, tracks, decoded):
    """Best first: decoded in full, most sectors, nearest its layout, then PC
    formats. A format that lacks a track holding data is not decoded in full."""
    results = {}
    for name, disk in disks.items():
        per = results[name] = {}
        for key, track in tracks.items():
            tdef = disk.track_map.get(key)
            if tdef is None:
                continue
            sig = signature(key, tdef)
            if sig not in decoded:
                decoded[sig] = decode(tdef, key, track)
            per[key] = decoded[sig]
    data = {k for per in results.values() for k, (f, _, _) in per.items() if f > 0}
    ranked = []
    for name, per in results.items():
        found = sum(f for f, _, _ in per.values())
        if not found:
            continue
        expected = sum(n for _, n, _ in per.values())
        off = sum(m for _, _, m in per.values()) / len(per)
        full = all(0 < n == f for f, n, _ in per.values()) and data <= per.keys()
        ranked.append(Match(full, found, expected, off, name))
    return sorted(ranked, key=lambda m: (not m.full, -m.found, round(m.misfit, 3),
                                         not m.name.startswith('ibm.'), m.name))


def decode(tdef, key, track):
    """Sectors found and expected, and how far they sit from the format's
    layout. Sectors the format does not expect count as missing: a format that
    would drop them does not fit. A track the image lacks is blank."""
    try:
        with quiet():
            t = tdef.mk_track(*key)
            if track is not None:
                t.decode_flux(track)
            return t.nsec - t.nr_missing(), t.nsec + len(strangers(t)), misfit(tdef, key, t)
    except Exception:
        return 0, 0, 0.0


def strangers(track):
    """Good sector IDs on an IBM track that its format does not have: gw's
    decoder reports them as unexpected and ignores them."""
    if not is_ibm(track):
        return set()
    known = {sector_id(s) for s in track.sectors}
    return {sector_id(s) for s in track.raw.sectors if s.idam.crc == 0} - known


def quiet():
    """gw's decoders remark on every sector a wrong format does not expect."""
    return contextlib.redirect_stdout(io.StringIO())


def misfit(tdef, key, track):
    """For an IBM-style track: how far, as a share of a revolution, the
    sectors gw found sit from where this format writes them, plus 5% if the
    index mark is not as the format writes it. 0 for other codecs."""
    if not is_ibm(track):
        return 0.0
    with quiet():
        template = tdef.mk_track(*key)
    where = {s.idam.r: s.start for s in template.sectors}
    per_rev = template.time_per_rev / template.clock
    seen = {}
    for s in track.raw.sectors:  # in order round the track, from the index
        if s.idam.crc == 0 and s.idam.r in where:
            seen.setdefault(s.idam.r, s.start)
    if not seen:
        return 0.0
    off = [abs(((seen[r] - where[r]) / per_rev + 0.5) % 1 - 0.5) for r in seen]
    index_mark = bool(track.raw.iams) != bool(template.iams)
    return sum(off) / len(off) + (0.05 if index_mark else 0.0)


def run(main):
    answers = queue.Queue()

    def listen():
        for line in sys.stdin:
            if not line.startswith('answer '):
                break
            answers.put(line[7:].rstrip('\n'))
        stop()  # asked to, or the app has gone

    def ask(prompt=''):
        print(ASK + json.dumps(prompt), file=sys.stderr, flush=True)
        while True:
            with contextlib.suppress(queue.Empty):
                return answers.get(timeout=0.1)  # stays interruptible

    builtins.input = ask
    signal.signal(signal.SIGINT, signal.default_int_handler)  # a GUI parent may ignore it
    try:
        threading.Thread(target=listen, daemon=True).start()
        code = main()
    except KeyboardInterrupt:
        code = 1
    sys.exit(code)


def steady_handshake():
    """gw waits for ever for the device's first reply. On Windows every
    second opening of the port can lose the first command, so the reply
    never comes; gw's own reset, run again, gets it through."""
    from greaseweazle import error, usb
    connect = usb.Unit.__init__

    def patient(unit, ser):
        # A device answers in milliseconds; asking again early does no harm.
        wait, ser.timeout = ser.timeout, 0.5
        try:
            for _ in range(3):
                with contextlib.suppress(struct.error):  # a short read: no reply
                    return connect(unit, ser)
            device = os.environ.get('FERRITEWEAZLE_DEVICE', 'Greaseweazle')
            raise error.Fatal(f'{device} interface did not answer.')
        finally:
            ser.timeout = wait

    usb.Unit.__init__ = patient


def bundled_caps():
    """Points gw at the SPS/CAPS library a package keeps beside its Python.
    gw's own search misses it, and once gw is updated looks elsewhere."""
    name = {'darwin': 'libcapsimage.dylib', 'win32': 'CAPSImg.dll'}.get(sys.platform, 'libcapsimage.so.5')
    path = os.path.join(sys.prefix, 'caps', name)
    if not os.path.isfile(path):
        return
    import ctypes
    from greaseweazle import error
    from greaseweazle.image import caps
    search = caps.open_libcaps

    def bundled():
        try:
            lib = ctypes.cdll.LoadLibrary(path)
        except OSError:
            return search()
        error.check(lib.CAPSInit() == 0, "Failure initialising CAPS/SPS library '%s'" % path)
        return lib

    caps.open_libcaps = bundled


# Adafruit's Greaseweazle-compatible firmware reports gw's hardware model 8,
# and clamps a seek to cylinder 79 while reporting success. ADAFRUIT_LAST
# must match device::adafruit::LAST_CYLINDER.
ADAFRUIT_MODEL = 8
ADAFRUIT_LAST = 79


def adafruit_seeks():
    """Stops a job that would seek an Adafruit RP2040 outside cylinders 0 to
    79. Its firmware would step to 79 instead, or take a negative cylinder as
    unsigned, and gw would read or write cylinder 79 under another number."""
    from greaseweazle import error, usb
    seek = usb.Unit.seek

    def within(unit, cyl, head):
        if unit.hw_model == ADAFRUIT_MODEL and not 0 <= cyl <= ADAFRUIT_LAST:
            raise error.Fatal(f'The Adafruit RP2040 reaches cylinders 0 to {ADAFRUIT_LAST}, not {cyl}.')
        return seek(unit, cyl, head)

    usb.Unit.seek = within


def read_passes(passes, disk, keep):
    """Makes `gw read` read up to `passes` times while sectors are missing, each
    pass's flux decoded into the tracks before, as gw's own retries do. Later
    passes read the tracks still missing sectors, or the whole `disk`. Pass N's
    flux goes to `keep` + ' N.scp', if `keep`."""
    from greaseweazle import track
    from greaseweazle.image.scp import SCP
    from greaseweazle.tools import read
    wanted = ('read_to_image', 'read_with_retry', 'read_and_normalise', 'print_summary')
    if not all(hasattr(read, f) for f in wanted):
        sys.exit('** FATAL ERROR:\nThis Greaseweazle Tools cannot read in passes.')
    first, retry = read.read_to_image, read.read_with_retry
    got, count, flux_of_pass = {}, {}, {}

    def read_with_retry(usb, args, t):
        capture = copy.copy(args)
        capture.raw = True  # keeps every retry's flux
        # Each read as gw makes it, before gw joins the next onto it in place.
        made, normalise = [], read.read_and_normalise

        def read_and_normalise(*a, **k):
            flux = normalise(*a, **k)
            with contextlib.suppress(Exception):
                each = copy.copy(flux)
                each.list, each.index_list = list(flux.list), list(flux.index_list)
                made.append(each)
            return flux

        read.read_and_normalise = read_and_normalise
        try:
            flux, dat = retry(usb, capture, t)
        finally:
            read.read_and_normalise = normalise
        key = t.cyl, t.head
        if keep:
            flux_of_pass[key] = flux
        count[key] = count.get(key, 0) + 1
        if key in got:
            old_flux, old = got[key]
            if old is not None:
                # Each read on its own, as gw decodes its retries.
                for each in made:
                    for pll in track.plls:
                        if old.nr_missing() == 0:
                            break
                        old.decode_flux(each, pll)
                print(f'T{t.cyl}.{t.head}: {old.summary_string()} from {count[key]} passes')
                dat = old
            if args.raw:
                old_flux.append(flux)
                flux = old_flux
        # Flux kept only to join the next pass's to, with --raw.
        got[key] = (flux if args.raw else None), dat
        return flux, dat

    def missing(t):
        dat = got.get((t.cyl, t.head), (None, None))[1]
        return dat is not None and dat.nr_missing() > 0

    def save(name):
        if not flux_of_pass:
            return
        os.makedirs(os.path.dirname(name), exist_ok=True)
        with SCP.to_file(name, None, False, {}) as image:
            for (cyl, head), flux in sorted(flux_of_pass.items()):
                image.emit_track(cyl, head, copy.copy(flux))  # cueing would clip ours

    def read_to_image(usb, args, image):
        if args.fmt_cls is None:
            return first(usb, args, image)
        if keep and isinstance(args.revs, float):
            args.revs = 2  # whole revolutions, as gw reads raw flux
        todo = None
        for n in range(1, passes + 1):
            flux_of_pass.clear()
            try:
                if todo is None:
                    first(usb, args, image)
                else:
                    print(f'Pass {n} of {passes}: {len(todo)} track' + 's' * (len(todo) != 1))
                    for t in todo:
                        # gw's name for it, which report_tracks may wrap.
                        flux, dat = read.read_with_retry(usb, args, t)
                        if args.raw:
                            image.emit_track(t.cyl, t.head, flux)
                        elif dat is not None:
                            image.emit_track(t.cyl, t.head, dat)
            finally:
                if keep:
                    save(f'{keep} {n}.scp')
            # gw's track iterator hands back one object, moved along.
            tracks = [copy.copy(t) for t in args.tracks]
            short = [t for t in tracks if missing(t)]
            if not short:
                break
            todo = tracks if disk else short
        if n > 1:
            read.print_summary(args, {k: d for k, (_, d) in got.items() if d is not None})

    read.read_to_image, read.read_with_retry = read_to_image, read_with_retry


# What the bridge has patched in gw, as it was, so that a gw whose insides
# differ from those the bridge knows runs as it is: see guarded.
PATCHED = []
NOT_OWN = object()


def patch(owner, name, value):
    """Sets `owner`'s `name` to `value`, noting what it was."""
    PATCHED.append((owner, name, vars(owner).get(name, NOT_OWN)))
    setattr(owner, name, value)


def guarded(install):
    """Runs `install`, which patches gw to report on it. Where that fails,
    as with a gw whose insides differ from those the bridge knows, undoes
    every patch so far: gw runs as it is, unreported, and the log says why."""
    try:
        install()
        return True
    except Exception as e:
        while PATCHED:
            owner, name, was = PATCHED.pop()
            if was is NOT_OWN:
                delattr(owner, name)
            else:
                setattr(owner, name, was)
        print(f'Ferriteweazle: no reports from this Greaseweazle Tools: {e!r}', file=sys.stderr)
        return False


def round_from_index(args):
    """Whether gw takes each track round from the disk's index, so that what
    the bridge reports of it lies where it lies on the disk: not with
    --reverse, which runs it backwards, nor --fake-index, which starts it
    wherever it falls."""
    return not getattr(args, 'reverse', False) and getattr(args, 'fake_index', None) is None


def report_tracks(command):
    """Has gw report each track it reads, converts or writes on a TRACK line,
    for the disk view: a read's flux and what it decoded, the input track a
    conversion decoded, and for a write, the flux gw writes as it is, or the
    master track it writes from a format's sectors, then the track as gw's
    verify reads it back. None where the track does not run round from the
    index. For a write, also whether gw verifies each track, on a VERIFY
    line as gw comes to write it."""
    if command in ('read', 'convert', 'write'):
        report_places()
    if command in ('read', 'convert'):
        scaling()
    if command == 'read':
        from greaseweazle.tools import read
        inner = read.read_with_retry
        joining()

        def read_with_retry(usb, args, t):
            flux, dat = inner(usb, args, t)
            if round_from_index(args):
                report(t.cyl, t.head, indexed(flux, args), dat)
            return flux, dat

        patch(read, 'read_with_retry', read_with_retry)
    elif command == 'convert':
        from greaseweazle.tools import convert
        opened, process = convert.open_input_image, convert.process_input_track
        took = {}

        def open_input_image(args, image_class):
            image = opened(args, image_class)
            get = image.get_track

            def get_track(cyl, head):
                took[cyl, head] = track = once(get(cyl, head))
                return track

            image.get_track = get_track
            return image

        def process_input_track(args, t, in_image):
            dat = process(args, t, in_image)
            track = took.pop((t.physical_cyl, t.physical_head), None)
            if round_from_index(args):
                report(t.cyl, t.head, track, dat, absent=track is None)
            if getattr(getattr(track, 'flux', None), 'once', False):
                del track.flux
            return dat

        patch(convert, 'open_input_image', open_input_image)
        patch(convert, 'process_input_track', process_input_track)
    elif command == 'write':
        from greaseweazle.codec import codec
        from greaseweazle.flux import Flux
        from greaseweazle.tools import write
        from greaseweazle.track import MasterTrack
        opened, writing = write.open_image, write.write_from_image
        taking = {}

        def write_from_image(usb, args, image, *a, **k):
            taking['args'] = args
            put = usb.write_track

            def write_track(*a, **k):
                done = put(*a, **k)
                # What gw wrote, once it has: once for the track, however often retried.
                pending = taking.pop('pending', None)
                if pending is not None and tuple(pending[:2]) == taking.get('track'):
                    report(*pending)
                return done

            with contextlib.suppress(Exception):
                usb.write_track = write_track
            return writing(usb, args, image, *a, **k)

        def written(*pending, master=True):
            # gw's write precompensation moves a master track's flux as it
            # writes it: then what it writes is not this.
            if not master or getattr(taking.get('args'), 'precomp', None) is None:
                taking['pending'] = pending

        def open_image(args, image_class):
            image = opened(args, image_class)
            get = image.get_track

            def get_track(cyl, head):
                taking['track'] = cyl, head
                track = get(cyl, head)
                args = taking.get('args')
                if args is None or not round_from_index(args):
                    return track
                if track is None:
                    report(cyl, head, None, None, 'image', absent=True)
                # gw writes raw flux as it is, from the index to the next, with no
                # format to decode it into; a codec's track as its master track.
                # A hard-sectored disk's waits on its holes.
                elif not isinstance(track, codec.Codec) and args.fmt_cls is None and not args.hard_sectors:
                    with contextlib.suppress(Exception):
                        flux = copy.copy(track.flux())
                        # gw writes only flux that starts at an index; one that
                        # runs on past it to a splice overwrites its own start.
                        if flux.index_cued and not flux.splice:
                            flux.set_nr_revs(1)
                            written(cyl, head, flux, None, 'image',
                                    master=isinstance(track, MasterTrack))
                return track

            image.get_track = get_track
            return image

        def decoded(flux, made):
            """`flux` decoded as the format being written has the track, or
            with none, as a track of the kind `made` is."""
            if (fmt := taking['args'].fmt_cls) is not None:
                return decoded_as(fmt, taking['track'], flux)
            return decoded_like(made, flux)

        def mastering(master):
            def mastered(self, *a, **k):
                if taking.get('mastering'):  # a codec's master track made by another's
                    return master(self, *a, **k)
                taking['mastering'] = True
                try:
                    track = master(self, *a, **k)
                finally:
                    taking['mastering'] = False
                # gw writes the master track from the index, scaled to the
                # drive's revolution. A hard-sectored disk's waits on its holes.
                args = taking.get('args')
                if 'track' in taking and args and round_from_index(args) and not args.hard_sectors:
                    with contextlib.suppress(Exception):
                        # Decoded as the disk turns on past the index, so that a
                        # sector over it reads to its end, as it does from the disk;
                        # one revolution of it as gw writes it.
                        turns = track.flux(revs=2)
                        one = copy.copy(turns)
                        one.set_nr_revs(1)
                        written(*taking['track'], one, decoded(turns, self), 'written')
                return track
            return mastered

        def verifying(verify):
            def verified(self, flux, *a, **k):
                ok = verify(self, flux, *a, **k)
                # What the disk holds now: the track read back, as gw's check read it.
                with contextlib.suppress(Exception):
                    if 'track' in taking and round_from_index(taking.get('args')):
                        report(*taking['track'], flux, decoded(flux, self), 'verify')
                return ok
            vars(verified)[REPORTS] = True
            return verified

        def writing_out(out):
            def flux_for_writeout(self, *a, **k):
                # gw verifies only a MasterTrack with a verify, unless
                # --no-verify (gw 1.23's test).
                with contextlib.suppress(Exception):
                    if 'track' in taking and (args := taking.get('args')) is not None:
                        c, h = taking['track']
                        verifies = (not args.no_verify and isinstance(self, MasterTrack)
                                    and self.verify is not None)
                        line = json.dumps({'c': c, 'h': h, 'verifies': verifies}, separators=(',', ':'))
                        print(VERIFY + line, flush=True)
                        # An image's own verifier, as an IPF's or EDSK's, reports what it reads back too.
                        if verifies and not vars(type(self.verify).verify_track).get(REPORTS):
                            cls = type(self.verify)
                            patch(cls, 'verify_track', verifying(cls.verify_track))
                return out(self, *a, **k)
            return flux_for_writeout

        patch(write, 'open_image', open_image)
        patch(write, 'write_from_image', write_from_image)
        patch(MasterTrack, 'flux_for_writeout', writing_out(MasterTrack.flux_for_writeout))
        patch(Flux, 'flux_for_writeout', writing_out(Flux.flux_for_writeout))
        for cls in codecs():
            if 'master_track' in vars(cls):
                patch(cls, 'master_track', mastering(cls.master_track))
            if 'verify_track' in vars(cls):
                patch(cls, 'verify_track', verifying(cls.verify_track))


def report_images(command):
    """Has gw report on IMAGE lines the image a job makes or takes tracks
    from: how gw lays out its file, each track it puts in a made image, the
    file once written, and a conversion's routes."""
    if command == 'read':
        from greaseweazle.tools import read
        patch(read, 'open_image', making(read.open_image))
    elif command == 'convert':
        from greaseweazle.tools import convert
        patch(convert, 'open_output_image', making(convert.open_output_image))
        patch(convert, 'open_input_image', taking_from(convert.open_input_image))
        patch(convert, 'convert', routing(convert.convert))
    elif command == 'write':
        from greaseweazle.tools import write
        patch(write, 'open_image', taking_from(write.open_image))


def image_line(event, **facts):
    print(IMAGE + json.dumps({'event': event, **facts}, separators=(',', ':')), flush=True)


def making(opened):
    """`opened`, which makes the image a job writes, made to report it."""
    def open_image(args, image_class):
        image = opened(args, image_class)
        with contextlib.suppress(Exception):
            layout = image_layout(image)
            image_line('open', role='made', file=getattr(image, 'filename', None),
                       type=type(image).__name__, layout=layout)
            emit, made = image.emit_track, image.get_image

            # Each track's sectors as the layout has them, by key.
            laid = {(t['c'], t['h']): [[s['i'], s['id'], s['len']] for s in t['sectors']]
                    for t in (layout or {}).get('tracks', [])}

            def emit_track(cyl, side, track):
                emit(cyl, side, track)
                # Where the file is laid out, what each track puts in it; and
                # whether its sectors are the layout's, as gw makes them from
                # the format, not an input image's own track passed through.
                if layout is not None:
                    with contextlib.suppress(Exception):
                        has = [bool(track.has_sec(i)) for i in range(track.nsec)]
                        held = bytes(track.get_img_track()).hex()
                        parts = [list(x) for x in sector_parts(track) or []]
                        facts = {} if parts == laid.get((cyl, side)) else {'laid': False}
                        image_line('track', c=cyl, h=side, has=has, bytes=held, **facts)

            def get_image():
                dat = made()
                with contextlib.suppress(Exception):
                    image_line('written', size=len(dat), tracks=as_laid_out(image, dat, False))
                return dat

            image.emit_track, image.get_image = emit_track, get_image
        return image
    return open_image


def taking_from(opened):
    """`opened`, which reads the image a job takes its tracks from, made to
    report it, checked against the file gw read it from."""
    def open_image(args, image_class):
        image = opened(args, image_class)
        with contextlib.suppress(Exception):
            image_line('open', **taken(image))
        return image
    return open_image


def taken(image):
    """What an image gw takes tracks from is: its file and gw's type, and
    how gw lays out the file, with its bytes, where the file is laid out so."""
    name = getattr(image, 'filename', None)
    layout = image_layout(image)
    size = held = None
    differs = False
    if layout is not None:
        with open(name, 'rb') as f:
            dat = f.read()
        size = len(dat)
        if as_laid_out(image, dat, True) is None:
            layout, differs = None, True
        else:
            laid = sum(p['len'] for t in layout['tracks'] for p in t['sectors'])
            held = dat[:laid].hex()
    out = {'role': 'source', 'file': name, 'type': type(image).__name__,
           'layout': layout, 'size': size, 'bytes': held}
    if differs:
        out['differs'] = True  # gw lays the type out, but not this file so
    return out


def image(args):
    """The image a write or a conversion is to take its tracks from, as its
    job reports it on opening: gw's own command run with `args`, as the job
    would run it, up to where gw has opened that image, before it opens a
    device or a conversion's output."""
    rest = list(args)
    while rest and rest[0].startswith('-'):  # gw's own options, such as --bt
        rest.pop(0)
    command = rest[0] if rest else None
    opener = {'write': 'open_image', 'convert': 'open_input_image'}.get(command)
    if opener is None:
        raise ValueError(f'gw {command} takes its tracks from no image.')
    tool = importlib.import_module('greaseweazle.tools.' + command)
    opened = getattr(tool, opener)

    def open_image(a, image_class):
        raise Captured(taken(opened(a, image_class)))

    from greaseweazle import track
    from greaseweazle.tools import util
    said, plls = io.StringIO(), track.plls[:]  # gw's convert adds --pll's to them
    usb_open = util.usb_open

    def no_device(*a, **k):
        raise ValueError('gw opens the Greaseweazle before this image.')

    # A gw that opens the device before the image must not open it here.
    setattr(tool, opener, open_image)
    util.usb_open = no_device
    try:
        with contextlib.redirect_stdout(said), contextlib.redirect_stderr(said):
            tool.main(['gw'] + rest)
    except Captured as c:
        return c.args[0]
    except SystemExit:
        # gw's parser has said why, last, after its name.
        lines = said.getvalue().strip().splitlines()
        why = lines[-1].split(' error: ', 1)[-1] if lines else 'gw took none of these arguments.'
        raise ValueError(why)
    finally:
        setattr(tool, opener, opened)
        util.usb_open = usb_open
        track.plls[:] = plls
    raise ValueError('gw opened no image.')


def routing(converting):
    """gw's conversion, made to report first each track it goes through, as
    its track lines name it, with where it lies in the input image and where
    in the output: gw's track lists can move them."""
    def convert(args, in_image, out_image):
        with contextlib.suppress(Exception):
            routes = [[t.cyl, t.head, *args.tracks.ch_to_pch(t.cyl, t.head),
                       t.physical_cyl, t.physical_head]
                      for t in args.out_tracks if (t.cyl, t.head) in args.tracks]
            image_line('routes', tracks=routes)
        return converting(args, in_image, out_image)
    return convert


def image_layout(image):
    """How gw lays out a sector image's file: each track in the file's
    order, keyed as gw keeps it, with the bytes each of its sectors takes, in
    turn, and the filler gw writes in place of each one it lacks; and the
    least cylinders the file has, past which gw writes cylinders up to the
    last it holds data in. None for an image gw keeps some other way, such
    as flux, or one this cannot lay out."""
    from greaseweazle.image.img import IMG
    if not isinstance(image, IMG):
        return None
    try:
        return sector_layout(image)
    except Exception:
        return None


def sector_layout(image):
    tracks, fillers = [], []
    for cyl, head in placed(image):
        track = image.fmt.mk_track(cyl, head)
        empty = bytes(track.get_img_track())
        parts = sector_parts(track)
        if not empty or parts is None or sum(n for _, _, n in parts) != len(empty):
            return None
        sectors, at = [], 0
        for index, sid, n in parts:
            filler = empty[at:at + n].hex()
            if filler not in fillers:
                fillers.append(filler)
            sectors.append({'i': index, 'id': sid, 'len': n, 'fill': fillers.index(filler)})
            at += n
        tracks.append({'c': cyl, 'h': head, 'sectors': sectors})
    return {'tracks': tracks, 'fillers': fillers, 'min_cyls': image.min_cyls}


def placed(image):
    """The tracks of a sector image in its file's order, keyed as gw keeps them."""
    for cyl, head in image.track_list():
        yield cyl, head ^ 1 if image.sides_swapped else head


def sector_parts(track):
    """Each of a track's sectors as gw puts them in a sector image, in turn:
    its index among the track's sectors, its C, H, R and N where it has them,
    and the bytes it takes. None where they are not all one size and gw has
    no other order for them."""
    if hasattr(track, 'sectors') and hasattr(track, 'img_bps'):
        # IBM-style: in the order of R, each padded to the format's bytes per sector.
        order = sorted(range(len(track.sectors)), key=lambda i: track.sectors[i].idam.r)
        size = lambda s: len(s.dam.data) if track.img_bps is None else track.img_bps
        return [(i, list(sector_id(track.sectors[i])), size(track.sectors[i])) for i in order]
    # Apple II's in its format's order, by their numbers; others' in their own.
    from greaseweazle.codec.apple2 import apple2_gcr
    if isinstance(track, apple2_gcr.Apple2GCR):
        order = list(track.config.secs)
    else:
        order = list(range(track.nsec))
    whole = len(track.get_img_track())
    if not order or whole % len(order):
        return None
    return [(i, None, whole // len(order)) for i in order]


def as_laid_out(image, dat, reading):
    """Each track's part of `dat`, an image's file, as [c, h, start, end], if
    every track there is gw's bytes for it, in turn, to the file's end: a
    file gw reads, padded as gw pads it, else one it writes, which ends early
    where gw left out the cylinders past the last it holds data in. None if
    they differ."""
    out, at = [], 0
    for cyl, head in placed(image):
        key = cyl, head
        track = image.to_track[key] if key in image.to_track else image.fmt.mk_track(*key)
        want = bytes(track.get_img_track())
        if not reading and at == len(dat):
            break
        got = dat[at:at + len(want)]
        if reading:
            got = got.ljust(len(want), b'\0')
        if got != want:
            return None
        out.append([cyl, head, at, at + len(want)])
        at += len(want)
    return out if reading or at == len(dat) else None


def codecs():
    """Every codec class gw has."""
    from greaseweazle.codec import codec
    todo, found = list(codec.Codec.__subclasses__()), []
    while todo:
        cls = todo.pop()
        found.append(cls)
        todo += cls.__subclasses__()
    return found


def report(cyl, head, track, dat, source=None, absent=False):
    """Prints a TRACK line for cyl.head: the flux of `track`, the sectors
    decoded into `dat`, a write's `source`, and `absent` where the image
    lacks the track. `turned` marks flux read from a disk or a flux image,
    not flux gw made. A report that fails is left out: gw must go on."""
    from greaseweazle.codec import codec
    from greaseweazle.flux import Flux
    with contextlib.suppress(Exception):
        out = {'c': cyl, 'h': head}
        if source:
            out['source'] = source
        if absent:
            out['absent'] = True
        if source in (None, 'verify') and isinstance(track, Flux):
            out['turned'] = True
        # A codec's track is an image's sectors, whose flux gw has yet to make.
        if track is not None and not isinstance(track, codec.Codec):
            with contextlib.suppress(Exception):
                out['flux'] = report_flux(track.flux())
        if isinstance(dat, codec.Codec):
            out['codec'] = report_codec(dat)
        print(TRACK + json.dumps(out, separators=(',', ':')), flush=True)


def report_flux(flux):
    """A track's flux, in ticks of its sample rate: each revolution read,
    from index pulse to index pulse, and their mean; and over each read gw
    joined into it, its flux transitions in each of BINS equal parts of a
    revolution, and the passes made over them, as shares of a revolution
    from the index: each revolution whole, and what the read took before its
    first pulse or after its last, by the length of the revolution beside
    it; in a read with none, by the others' mean, else by gw's measure of
    the drive's turn, `drive`. Whether gw scaled its times, `scaled`, as
    with --adjust-speed, and whether its pulses are a hard-sectored disk's
    first sector's hole, `holes`, as gw takes it, not its index. None where
    the pulses are not a disk's index: one of no length, or a read that
    runs two revolutions past one."""
    ends = list(itertools.accumulate(flux.list))
    bins, passes, revs = [0] * BINS, [], []
    # Each read's first value, which runs from where the read starts.
    firsts = [bisect.bisect_right(ends, start) for start, _, _ in reads(flux, 0.0)]
    split = list(reads(flux, ends[-1] if ends else 0.0))
    # Never gw's own mean, which counts the joints between reads.
    found = [b - a for _, pulses, _ in split for a, b in zip(pulses, pulses[1:])]
    measure = sum(found) / len(found) if found else getattr(flux, '_ticks_per_rev', None)
    for start, pulses, end in split:
        if not pulses or (len(pulses) < 2 and measure is None):
            continue  # no index, or no revolution, to place it by
        turns = [b - a for a, b in zip(pulses, pulses[1:])]
        first = turns[0] if turns else measure
        last = turns[-1] if turns else measure
        lead, tail = pulses[0] - start, end - pulses[-1]
        if min(turns + [first, last]) <= 0 or lead >= 2 * first or tail >= 2 * last:
            return None
        # Each part of the read: where it starts and ends, in ticks, the share
        # of a revolution it starts at, and the revolution's length.
        parts = [(a, b, 0.0, b - a) for a, b in zip(pulses, pulses[1:])]
        if lead > 0:
            parts.insert(0, (start, pulses[0], 1.0 - lead / first, first))
        if tail > 0:
            parts.append((pulses[-1], end, 0.0, last))
        for a, b, share, turn in parts:
            passes.append([share, share + (b - a) / turn])
            part = math.floor(share * BINS)
            at = bisect.bisect_right(ends, a)
            while True:
                edge = min(a + ((part + 1) / BINS - share) * turn, b)
                hit = bisect.bisect_right(ends, edge, at)
                bins[part % BINS] += hit - at
                at, part = hit, part + 1
                if edge >= b:
                    break
        revs += turns
    out = {'freq': flux.sample_freq, 'period': sum(revs) / len(revs) if revs else measure,
           'revs': revs, 'passes': passes, 'bins': bins, 'intervals': intervals(flux, firsts)}
    if not revs:
        out['drive'] = True
    if vars(flux).get(SCALED):
        out['scaled'] = True
    if getattr(flux, 'sector_list', None) is not None:
        out['holes'] = True
    return out


def intervals(flux, firsts):
    """How far apart the flux transitions of `flux` are, as gw holds them:
    its values, but those at `firsts`, which run from where a read starts,
    counted in bins `width` sample ticks wide, from bin `first`, the first
    with any, to the last before bin `top`, the first at or past
    INTERVAL_TOP, and how many are in it or past it."""
    width = max(1, round(flux.sample_freq * INTERVAL_BIN))
    # Rounded first: 40 MHz's 20 us is 800.0000000000001 ticks in floats.
    top = math.ceil(round(flux.sample_freq * INTERVAL_TOP / width, 6))
    # Each value counted, then each count binned: far fewer bins than values.
    values = collections.Counter(flux.list)
    values.subtract(flux.list[i] for i in firsts if i < len(flux.list))
    counts = collections.Counter()
    for value, n in values.items():
        counts[value // width] += n
    counts = +counts  # none of no count
    held = [int(b) for b in counts if b < top]
    first, last = min(held, default=0), max(held, default=-1)
    return {'width': width, 'first': first, 'top': top,
            'counts': [counts.get(b, 0) for b in range(first, last + 1)],
            'longer': sum(n for b, n in counts.items() if b >= top)}


def reads(flux, end):
    """Each read gw joined into `flux`, in ticks from its start: where it
    starts, its index pulses, and where it ends, `end` for the last."""
    joins = {k: (tail, cued) for k, tail, cued in vars(flux).get(JOINS, [])}
    start, at = 0.0, 0.0
    pulses = [0.0] if flux.index_cued else []
    for k, ticks in enumerate(flux.index_list):
        if k in joins:
            tail, cued = joins[k]
            yield start, pulses, at + tail
            start = at + tail
            pulses = [start] if cued else []
        at += ticks
        pulses.append(at)
    yield start, pulses, end


def joining():
    """Has gw's flux note where it joins one read onto the end of another:
    the index interval that runs from the first's last pulse over the rest of
    it and the start of the second, how far the first ran on past its last
    pulse, and whether the second starts at a pulse. gw counts the joint as a
    revolution; it is none."""
    from greaseweazle.flux import Flux
    append = Flux.append

    def appended(self, flux):
        with contextlib.suppress(Exception):
            k = len(self.index_list)
            factor = self.sample_freq / flux.sample_freq
            ran = sum(self.list) - sum(self.index_list)
            joins = vars(self).get(JOINS, []) + [(k, ran, flux.index_cued)]
            joins += [(k + j, t * factor, c) for j, t, c in vars(flux).get(JOINS, [])]
        append(self, flux)
        with contextlib.suppress(NameError):
            vars(self)[JOINS] = joins
            if vars(flux).get(SCALED):
                vars(self)[SCALED] = True
    patch(Flux, 'append', appended)


def scaling():
    """Has gw's flux note that gw scaled its times, as --adjust-speed does:
    then they are not the disk's."""
    from greaseweazle.flux import Flux
    scale = Flux.scale

    def scaled(self, *a, **k):
        vars(self)[SCALED] = True
        return scale(self, *a, **k)
    patch(Flux, 'scale', scaled)


def indexed(flux, args):
    """A read's flux with only its index pulses: gw tells a hard-sectored
    disk's sector holes from its index only in flux it does not keep raw.
    None where it cannot be told."""
    if not getattr(args, 'hard_sectors', False) or getattr(flux, 'sector_list', None) is not None:
        return flux
    if vars(flux).get(JOINS):
        return None
    with contextlib.suppress(Exception):
        flux = copy.copy(flux)  # identify_hard_sectors rebinds its lists
        flux.identify_hard_sectors()
        return flux
    return None


def once(track):
    """An image's track, its flux made once however often gw asks for it:
    gw's decoder asks, then report() does. Until the `flux` it is given,
    marked `once`, is taken off it again. Raw flux is its own already."""
    from greaseweazle.flux import Flux
    if track is None or isinstance(track, Flux) or not callable(getattr(track, 'flux', None)):
        return track
    make, made = track.flux, {}

    def flux(*a, **k):
        try:
            key = a, tuple(sorted(k.items()))
            hash(key)
        except TypeError:
            return make(*a, **k)
        if key not in made:
            made[key] = make(*a, **k)
        return made[key]
    flux.once = True
    with contextlib.suppress(Exception):  # a track that takes no attribute is left as it is
        track.flux = flux
    return track


def report_codec(dat):
    """What gw decoded of a track: its summary, timing and sectors. An IBM-
    style track has the sectors found round it as they lie, in bit cells
    from the index and, where decoded from flux, in seconds from it, with
    their data, those its format lays out, and the headers and data blocks
    found apart, which gw drops; others have the sectors that decoded, with
    their data and where report_places saw each begin."""
    from greaseweazle.codec.ibm import ibm
    inner = getattr(dat, 'track', dat)  # a scan's, once it has found one
    # A scan that found nothing keeps gw's stand-in timing, for its master track.
    timed = not isinstance(inner, ibm.IBMTrack_Empty)
    out = {'summary': dat.summary_string(), 'nsec': dat.nsec,
           'good': [s for s in range(dat.nsec) if dat.has_sec(s)],
           'time_per_rev': getattr(inner, 'time_per_rev', None) if timed else None,
           'clock': getattr(inner, 'clock', None) if timed else None}
    if hasattr(inner, 'iams'):
        raw = getattr(inner, 'raw', None)
        # A fixed format's track holds what was found apart from its layout;
        # a scan's or an image's track only what it holds.
        decoded = hasattr(raw, 'clock')
        found = raw if decoded else inner
        out['iams'] = [a.start for a in found.iams]
        out['iam_times'] = [vars(a).get(WHEN) for a in found.iams]
        out['found'] = [ibm_sector(s) for s in found.sectors]
        out['apart'] = getattr(found, APART, [])
        if timed:
            out['mode'] = str(inner.mode)
        out['decodes'] = getattr(found, DECODES, [])
        if decoded:  # its data is that of the sectors found
            out['laid'] = [ibm_sector(s, False) for s in inner.sectors]
    else:
        out['places'] = getattr(dat, PLACES, {})
        out['data'] = image_data(dat)
    return out


def image_data(track):
    """A track's sectors that decoded, by number, each as gw puts it in a
    sector image: without a Mac sector's tag bytes or an AmigaDOS sector's
    label, which gw's images do not keep."""
    parts = sector_parts(track)
    if parts is None:
        return {}
    held, at, out = bytes(track.get_img_track()), 0, {}
    for i, _, n in parts:
        if track.has_sec(i):
            out[i] = held[at:at + n].hex()
        at += n
    return out


def ibm_sector(s, data=True):
    sector = {'id': list(sector_id(s)), 'start': s.start, 'header_end': s.idam.end,
              'data_start': s.dam.start, 'end': s.end, 'header': s.idam.crc == 0,
              'data': s.dam.crc == 0, 'mark': s.dam.mark}
    head, body = vars(s.idam).get(WHEN), vars(s.dam).get(WHEN)
    if head and body:
        sector['times'], sector['turn'] = [head[0], head[1], body[0], body[1]], head[2]
    if (found := vars(s).get(COPY)) is not None:
        sector['copy'] = found
    if data and s.dam.data:
        sector['bytes'] = bytes(s.dam.data).hex()
    return sector


# Attributes report_places, joining and scaling set on gw's objects.
PLACES = 'ferriteweazle_places'
APART = 'ferriteweazle_apart'
DECODES = 'ferriteweazle_decodes'
COPY = 'ferriteweazle_copy'
FLUX = 'ferriteweazle_flux'
FLUXES = itertools.count()
TIMES = 'ferriteweazle_times'
STARTS = 'ferriteweazle_starts'
JOINS = 'ferriteweazle_joins'
SCALED = 'ferriteweazle_scaled'
# Marks gw's verify_track as reporting what it reads back.
REPORTS = 'ferriteweazle_reports'
BASE = 'ferriteweazle_base'
WHEN = 'ferriteweazle_when'


class Clock:
    """When each of a PLL track's bit cells starts, in seconds from its
    first, and when its last ends, as `clock[cell]`: by its clock as it
    followed the flux, which gw scales to the format's revolution. Summed a
    block at a time: a float for every cell would take megabytes a track."""
    BLOCK = 1024

    def __init__(self, cells):
        self.cells = cells
        sums = (sum(cells[i:i + self.BLOCK]) for i in range(0, len(cells), self.BLOCK))
        self.sums = list(itertools.accumulate(sums, initial=0.0))

    def __getitem__(self, cell):
        block = cell // self.BLOCK
        return self.sums[block] + sum(self.cells[block * self.BLOCK:cell])


def times(pll):
    """A PLL track's Clock."""
    kept = vars(pll).get(TIMES)
    if kept is None:
        kept = vars(pll)[TIMES] = Clock(pll.timearray)
    return kept


def indexes(pll):
    """The bit cells of a PLL track its revolutions start at, from its first,
    and where the last ends: each but the first an index pulse, and the first
    too if the flux starts at one."""
    kept = vars(pll).get(STARTS)
    if kept is None:
        kept = vars(pll)[STARTS] = list(itertools.accumulate((r.nr_bits for r in pll.revolutions),
                                                             initial=0))
    return kept


def decoded_from(raw, areas, nr, flux):
    """Decode `nr` of an IBM track, as gw's decoder found `areas` on PLL track
    `raw` from `flux`: which flux, by number, as gw decodes one flux again
    with another PLL; each revolution's bit cells, index to index, and those
    after the last index; and each area in turn, a list: its kind, the
    revolution it lies in, its place in bit cells from that revolution's
    index, and what gw read of it. Each area keeps the decode and its place
    in its list, as COPY."""
    from greaseweazle.codec.ibm import ibm
    starts = indexes(raw)
    out, listed = [], []
    for x in sorted(areas, key=lambda x: vars(x).get(BASE, 0) + x.start):
        base = vars(x).get(BASE, 0)
        rev = max(bisect.bisect_right(starts, base + x.start) - 1, 0)
        # gw counts an area from the index before it, but for one past a
        # revolution holding none, from the one before that.
        at = base - starts[rev]
        if isinstance(x, ibm.Sector):
            i, d = x.idam, x.dam
            area = [1, rev, i.start + at, i.end + at, d.start + at, d.end + at,
                    int(i.crc == 0), int(d.crc == 0), d.mark, i.c, i.h, i.r, i.n]
        elif isinstance(x, ibm.IDAM):
            area = [2, rev, x.start + at, x.end + at, int(x.crc == 0), x.c, x.h, x.r, x.n]
        elif isinstance(x, ibm.DAM):
            area = [3, rev, x.start + at, x.end + at, x.mark]
        elif isinstance(x, ibm.IAM):
            area = [0, rev, x.start + at, x.end + at]
        else:
            continue
        listed.append(x)
        out.append(area)
    if FLUX not in vars(flux):
        vars(flux)[FLUX] = next(FLUXES)
    decode = {'flux': vars(flux)[FLUX], 'cells': [b - a for a, b in zip(starts, starts[1:])],
              'tail': len(raw.bitarray) - starts[-1], 'areas': out}
    # Only once nothing can fail, so that no area names a decode left out.
    for i, x in enumerate(listed):
        vars(x)[COPY] = [nr, i]
    return decode


def report_places():
    """Has gw's decoders note what they find but do not keep: where a
    non-IBM codec's sectors lie by its PLL's clock (PLACES), when each IBM
    area lies from its revolution's index (WHEN), IBM headers and data found
    apart (APART), and each IBM decode's areas (DECODES)."""
    import bitarray as module
    from bitarray import bitarray
    from greaseweazle import track
    # gw's codec module first: it imports the codecs in an order their own
    # imports of each other allow.
    from greaseweazle.codec import codec  # noqa: F401
    from greaseweazle.codec.ibm import ibm
    seen = {}
    # bitarray 3 searches lazily, as gw 1.23 takes it; 2, as older gw, a list.
    lazy = int(module.__version__.split('.')[0]) >= 3

    class Offsets(list):
        """A search's list of offsets, as bitarray 2 gives it, each noted as
        it is read."""

        def __iter__(self):
            return noted(self.bits, list.__iter__(self))

    def noted(bits, offsets):
        for offs in offsets:
            if bits is current('bits'):
                seen['at'], seen['parts'] = bits.base + offs, []
            yield offs

    def current(name):
        ref = seen.get(name)
        return ref() if ref is not None else None

    class Searched(bitarray):
        """A PLL track's bits, or a revolution's, `base` bits into it, which
        note the decoder's searches and reads: not those of a part of them,
        such as its search for a data mark."""

        def search(self, *a, **k):
            found = super().search(*a, **k)
            if lazy:
                return noted(self, found)
            offsets = Offsets(found)
            offsets.bits = self
            return offsets

        def __getitem__(self, key):
            if isinstance(key, slice) and self is current('bits'):
                start, stop, _ = key.indices(len(self))
                seen['parts'].append((self.base + start, self.base + stop))
            return super().__getitem__(key)

    def searched(pll, bits, base):
        bits = Searched(bits)
        # Held weakly: a PLL track holds every cell of the track.
        seen['bits'], seen['pll'] = weakref.ref(bits), weakref.ref(pll)
        bits.base, seen['parts'] = base, []
        seen.pop('at', None)
        return bits

    begin, get, get_rev = (track.PLLTrack.__init__, track.PLLTrack.get_all_data,
                           track.PLLTrack.get_revolution)

    def init(self, *a, **k):
        seen.clear()
        begin(self, *a, **k)
        data = k['data'] if 'data' in k else a[1]
        # Only raw flux may start between index pulses; a bitcell track's flux starts at one.
        seen['cued'] = getattr(data, 'index_cued', True)

    def get_all_data(self):
        bits, cells = get(self)
        return searched(self, bits, 0), cells

    def get_revolution(self, nr):
        bits, cells = get_rev(self, nr)
        return searched(self, bits, sum(r.nr_bits for r in self.revolutions[:nr])), cells

    def noting(add):
        def noted(self, sec_id, *a, **k):
            parts = seen.get('parts')
            with contextlib.suppress(Exception):
                if parts:
                    # In seconds by the PLL's clock: with no sync searched to, as
                    # in a hard sector, from its data.
                    at = seen.get('at', parts[-1][0])
                    end = max(e for b, e in parts if b >= at)
                    pll = current('pll')
                    clock, starts = times(pll), indexes(pll)
                    place = {'at': clock[at], 'end': clock[end],
                             'revs': [clock[b] - clock[a] for a, b in zip(starts, starts[1:])],
                             'cued': seen['cued']}
                    if parts[-1][0] > at:
                        place['data'] = clock[parts[-1][0]]
                    vars(self).setdefault(PLACES, {}).setdefault(sec_id, place)
            seen['parts'] = []
            seen.pop('at', None)
            return add(self, sec_id, *a, **k)
        return noted

    def keeping(decode):
        def kept(raw, *a, **k):
            seen['mmfm'] = []
            seen['areas'] = areas = decode(raw, *a, **k)
            with contextlib.suppress(Exception):
                when(raw, areas, seen.pop('mmfm'))
            return areas
        return staticmethod(kept)

    def when(raw, areas, mmfm):
        """Notes on each of an IBM track's areas where in time it lies, from
        the index its revolution starts at, and that revolution's length: by
        the clock of `raw`, the PLL track it was found on, and for a DEC RX02
        data block, its end by the clock of the double-rate track it was
        decoded from."""
        clock, starts = times(raw), indexes(raw)
        late = iter(mmfm)
        for x in sorted(areas, key=lambda x: x.start + vars(x).get(BASE, 0)):
            # gw's base is the revolution it searched from; an area past it
            # lies in the revolution whose index it follows.
            cell = vars(x).get(BASE, 0) + x.start
            at = max(bisect.bisect_right(starts, cell) - 1, 0)
            base = starts[at]
            turn = clock[starts[at + 1]] - clock[base] if at + 1 < len(starts) else None
            for y in [x] + [getattr(x, n) for n in ('idam', 'dam') if hasattr(x, n)]:
                start = vars(x).get(BASE, 0) + y.start
                end = vars(x).get(BASE, 0) + y.end
                vars(y)[WHEN] = [clock[start] - clock[base], clock[end] - clock[base], turn]
            if isinstance(x, ibm.Sector) and (x.dam.mark & 0xfb) == ibm.Mark.DDAM_DEC_MMFM:
                pll, end = next(late)
                vars(x)[WHEN][1] = vars(x.dam)[WHEN][1] = times(pll)[end] - clock[base]

    area_delta = ibm.TrackArea.delta

    def delta(self, d):
        # Its place in its revolution, d bit cells into the PLL track.
        area_delta(self, d)
        vars(self)[BASE] = d

    mmfm_decode = ibm.dec_mmfm.decode

    def mmfm(bits):
        # The double-rate bits of a DEC RX02 data block, the last part read of them.
        parts = seen.get('parts')
        if parts:
            seen.setdefault('mmfm', []).append((current('pll'), parts[-1][1]))
        return mmfm_decode(bits)

    raw_decode = ibm.IBMTrack.decode_raw

    def decode_raw(self, raw, pll, flux):
        raw_decode(self, raw, pll, flux)
        areas = seen.pop('areas', [])
        decodes = vars(self).setdefault(DECODES, [])
        with contextlib.suppress(Exception):
            decodes.append(decoded_from(raw, areas, len(decodes), flux))
        with contextlib.suppress(Exception):
            apart = vars(self).setdefault(APART, [])
            for x in areas:
                if isinstance(x, ibm.IDAM):
                    block = {'id': [x.c, x.h, x.r, x.n], 'start': x.start, 'end': x.end,
                             'header': x.crc == 0}
                elif isinstance(x, ibm.DAM):
                    block = {'start': x.start, 'end': x.end, 'mark': x.mark}
                else:
                    continue
                if (at := vars(x).get(WHEN)):
                    block['times'], block['turn'] = at[:2], at[2]
                if (found := vars(x).get(COPY)) is not None:
                    block['copy'] = found
                apart.append(block)

    patch(track.PLLTrack, '__init__', init)
    patch(track.PLLTrack, 'get_all_data', get_all_data)
    patch(track.PLLTrack, 'get_revolution', get_revolution)
    patch(ibm.IBMTrack, 'mfm_decode_raw', keeping(ibm.IBMTrack.mfm_decode_raw))
    patch(ibm.IBMTrack, 'fm_decode_raw', keeping(ibm.IBMTrack.fm_decode_raw))
    patch(ibm.IBMTrack, 'decode_raw', decode_raw)
    patch(ibm.TrackArea, 'delta', delta)
    patch(ibm.dec_mmfm, 'decode', mmfm)
    for cls in codecs():
        if 'add' in vars(cls):
            patch(cls, 'add', noting(cls.add))


def gw(args):
    from greaseweazle import cli
    steady_handshake()
    adafruit_seeks()
    passes = int(os.environ.get('FERRITEWEAZLE_PASSES') or 1)  # these must match app.rs's pass_env
    if passes > 1:
        reread = os.environ.get('FERRITEWEAZLE_REREAD') == 'disk'
        read_passes(passes, reread, os.environ.get('FERRITEWEAZLE_KEEP'))
    # gw's own options, such as --bt, come before its command.
    command = next((a for a in args if not a.startswith('-')), None)
    guarded(lambda: (report_tracks(command), report_images(command)))
    sys.argv = ['gw'] + args
    return cli.main()


def detect_like_gw(args):
    """Runs detect with its output on stderr and errors reported as gw does."""
    sys.stdout = sys.stderr
    try:
        steady_handshake()
        adafruit_seeks()
        return detect(args)
    except Exception as e:
        print('** FATAL ERROR:\n' + str(e))
        return 1


def latest(repo=GW_REPO):
    """The tag of a repository's newest release. GitHub leaves out prereleases."""
    import requests
    reply = requests.get(f'{GITHUB_API}/repos/{repo}/releases/latest', timeout=(5, 15))
    if reply.status_code == 404:
        raise ValueError(f'{repo} has no release on GitHub.')
    reply.raise_for_status()
    return reply.json()['tag_name']


def fetch(tag, name, folder):
    """Downloads Ferriteweazle's release asset `name` into folder, checked
    against the release's SHA-256 sums, and returns what to install: the file,
    or folder/unpacked for a zip or tarball."""
    import hashlib, requests, shutil, tarfile, zipfile
    base = f'{GITHUB}/{APP_REPO}/releases/download/{tag}'
    sums = requests.get(f'{base}/SHA256SUMS-{tag.lstrip("v")}.txt', timeout=(5, 30))
    sums.raise_for_status()
    wanted = dict(reversed(line.split()) for line in sums.text.splitlines() if line.strip())
    path, digest = os.path.join(folder, name), hashlib.sha256()
    with requests.get(f'{base}/{name}', stream=True, timeout=(5, 60)) as reply:
        reply.raise_for_status()
        with open(path, 'wb') as f:
            for chunk in reply.iter_content(1 << 16):
                digest.update(chunk)
                f.write(chunk)
    if wanted.get(name) != digest.hexdigest():
        raise ValueError(f'{name} does not match its SHA-256 in the release.')
    if name.endswith(('.zip', '.tar.gz')):
        unpacked = os.path.join(folder, 'unpacked')
        shutil.rmtree(unpacked, ignore_errors=True)
        with (zipfile.ZipFile if name.endswith('.zip') else tarfile.open)(path) as archive:
            archive.extractall(unpacked)
        return unpacked
    return path


def source(tag):
    import requests, zipfile
    reply = requests.get(f'{GITHUB}/{GW_REPO}/archive/refs/tags/{tag}.zip', timeout=(5, 60))
    reply.raise_for_status()
    return zipfile.ZipFile(io.BytesIO(reply.content))


def compiled_from(z):
    """What gw's compiled part is made from: its C code, and what setup.py needs."""
    code = {n.split('/', 1)[1]: z.read(n) for n in z.namelist()
            if '/src/greaseweazle/optimised/' in n and n.endswith(('.c', '.h'))}
    setup = next(n for n in z.namelist() if n.count('/') == 1 and n.endswith('/setup.py'))
    needs = re.search(r'install_requires\s*=\s*\[(.*?)\]', z.read(setup).decode(), re.S)
    return code, needs and needs.group(1).split()


def update(tag, bundled, folder):
    """Installs gw `tag` in folder/tag. It keeps the bundled gw's compiled part,
    so it must be made from the same C code and need the same packages."""
    import compileall, shutil
    from greaseweazle import optimised
    new = source(tag)
    if compiled_from(new) != compiled_from(source(bundled)):
        raise ValueError(f'gw {tag.lstrip("v")} changes its C code or its dependencies, '
                         'so it needs a new build of Ferriteweazle.')
    part, done = os.path.join(folder, tag + '.part'), os.path.join(folder, tag)
    shutil.rmtree(part, ignore_errors=True)
    for name in new.namelist():
        rel = name.partition('/src/')[2]
        if rel.startswith('greaseweazle/') and not name.endswith('/'):
            path = os.path.join(part, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, 'wb') as f:
                f.write(new.read(name))
    with open(os.path.join(part, 'greaseweazle', '__init__.py'), 'w') as f:
        f.write(f"__version__ = '{tag.lstrip('v')}'\n")  # setup.py writes this for pip
    built = os.path.dirname(optimised.__file__)
    for name in os.listdir(built):
        if name.startswith('optimised.') and name.endswith(('.so', '.pyd')):
            shutil.copy2(os.path.join(built, name), os.path.join(part, 'greaseweazle', 'optimised'))
    compileall.compile_dir(part, quiet=1)
    shutil.rmtree(done, ignore_errors=True)
    os.replace(part, done)
    return tag


def reported(work):
    """Prints what `work` returns, or exits with its error for the app to show."""
    import requests
    try:
        print(work(), flush=True)
    except requests.Timeout:
        sys.exit('GitHub did not answer in time.')
    except requests.ConnectionError:
        sys.exit('Could not reach GitHub.')
    except Exception as e:
        sys.exit(str(e) or type(e).__name__)


if __name__ == '__main__':
    mode, args = sys.argv[1], sys.argv[2:]
    if mode in ('serve', 'run', 'detect'):
        bundled_caps()
    {'serve': serve,
     'run': lambda: run(lambda: gw(args)),
     'detect': lambda: run(lambda: detect_like_gw(args)),
     'latest': lambda: reported(lambda: latest(*args)),
     'fetch': lambda: reported(lambda: fetch(*args)),
     'update': lambda: reported(lambda: update(*args))}[mode]()
