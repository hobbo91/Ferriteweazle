"""Ferriteweazle's link to gw, through gw's own modules. gw is left as it is,
but for a time limit on the first reply from the device (steady_handshake).

  python bridge.py serve        one JSON request per stdin line, one reply per stdout line
  python bridge.py run ARGS     runs `gw ARGS`; stdin takes 'answer TEXT', anything else stops
  python bridge.py detect ARGS  finds the format of the disk in the drive, or of a flux file
  python bridge.py latest [REPO] prints the tag of the newest release of REPO (gw's) on GitHub
  python bridge.py update TAG BUNDLED DIR
                                installs gw TAG in DIR/TAG, beside the bundled gw BUNDLED
  python bridge.py fetch TAG NAME DIR
                                downloads Ferriteweazle's release asset NAME into DIR, checked
                                against the release's SHA256SUMS, unpacking a zip or tarball
"""
import argparse, builtins, contextlib, functools, importlib, io, json, os, queue, re, signal, struct, sys, threading, typing, _thread

# Must match job.rs.
ASK = '@ferriteweazle ask '
RESULT = '@ferriteweazle result '

# gw on GitHub; the variables point tests at a server of their own.
GITHUB = os.environ.get('FERRITEWEAZLE_GITHUB', 'https://github.com')
GITHUB_API = os.environ.get('FERRITEWEAZLE_GITHUB_API', 'https://api.github.com')
GW_REPO = 'keirf/greaseweazle'
APP_REPO = 'hobbo91/ferriteweazle'


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
    return {'name': name, 'about': mod.description,
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
    """The options class behind `file.ext::opt=val`."""
    import inspect
    from greaseweazle.image.image import ImageOpts
    for k in cls.__mro__:
        t = inspect.get_annotations(k).get('opts')
        t = getattr(sys.modules[k.__module__], t, None) if isinstance(t, str) else t
        if isinstance(t, type):
            return t
    mod = sys.modules[cls.__module__]
    own = [v for v in vars(mod).values() if isinstance(v, type)
           and issubclass(v, ImageOpts) and v.__module__ == mod.__name__]
    return own[0] if len(own) == 1 else type(cls.opts)


def settings(o, names):
    """File options with their defaults. A bool default marks a flag, which gw
    takes as `::name` since any value it is given counts as true."""
    try:
        inst = o()
    except Exception:
        inst = None
    defaults = [getattr(inst, n, None) for n in names]
    return [{'name': n, 'default': d if type(d) in (bool, int, float, str) else None}
            for n, d in zip(names, defaults)]


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
    """The format gw takes from an image when none is chosen, opening it as
    write and convert do: its type's own, or one found in the file. None if
    one must be chosen."""
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
    """The formats a disk definitions file adds, and gw's error for each one
    it cannot use."""
    from greaseweazle.codec import codec
    if not os.path.isfile(path):
        raise ValueError('There is no such file.')
    names = formats(path)
    errors = []
    for name in names:
        try:
            with quiet():
                codec.get_diskdef(name, path)
        except Exception as e:
            errors.append(str(e) or type(e).__name__)
    if not names:
        errors.append('It defines no disks.')
    return {'formats': names, 'errors': errors}


def format_info(name, diskdefs=None):
    from greaseweazle.codec import codec
    d = codec.get_diskdef(name, diskdefs)
    if d is None:
        raise ValueError(f'unknown format: {name}')
    tracks = [t for c in range(d.cyls) for h in range(d.heads) if (t := d.mk_track(c, h))]
    info = {'cyls': d.cyls, 'heads': d.heads}
    if tracks:
        # A scan's tracks are empty until read: IBM, of any layout.
        info['encoding'] = re.sub(r'\s*(\(.*|Empty)$', '', tracks[0].summary_string())
        info['sectors'] = [min(t.nsec for t in tracks), max(t.nsec for t in tracks)]
        with contextlib.suppress(Exception):
            info['bytes'] = sum(len(t.get_img_track()) for t in tracks)
    return info


def fits(ext, name, diskdefs=None):
    """gw's objection to an image of type `ext` in format `name`, or None. It
    is made in memory from the format's own tracks, as a read makes it, and
    read back where gw reads the type as sectors."""
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
        # Some fail on an assertion, which says nothing.
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
             'serial': p.serial_number, 'score': s,
             'denied': linux and not os.access(p.device, os.R_OK | os.W_OK)}
            for s, p in found]


@functools.cache
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
           'fits': fits}
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
    """Finds the format of the disk in the drive, or of a flux image, and
    prints the formats that read it in full, best first, after RESULT."""
    from greaseweazle.tools import util
    p = argparse.ArgumentParser(prog='detect')
    p.add_argument('--device')
    p.add_argument('--drive', default='A')
    p.add_argument('--diskdefs')
    p.add_argument('file', nargs='?')
    a = p.parse_args(argv)
    found = []
    if a.file:
        # First: gw 1.23's track image modules import it in a circle.
        importlib.import_module('greaseweazle.codec.codec')
        image = util.get_image_class(a.file).from_file(a.file, None, {})
        found.append(probe(image.get_track, a.diskdefs))
    else:
        usb = util.usb_open(a.device)

        def read(c, h):
            usb.seek(c, h)
            flux = usb.read_track(revs=2)
            print(f'T{c}.{h}: {flux.summary_string()}')
            return flux

        util.with_drive_selected(lambda: found.append(probe(read, a.diskdefs)), usb,
                                 util.Drive()(a.drive))
    ranked, step = found[0]
    whole = [m.name for m in ranked if m.full]
    for m in ranked[:6]:
        fit = f', {m.misfit:.1%} from its layout' if m.full else ', not all'
        print(f'{m.name}: {m.found} of {m.expected} sectors{fit}')
    if step > 1:
        print('This is a 40-track disk in an 80-track drive: it needs double step.')
    print(RESULT + json.dumps({'formats': whole, 'step': step}), flush=True)
    if not whole:
        print('** FATAL ERROR:\nNo format gw knows reads this disk in full. Choose one by hand.')
        return 1
    print(f'Format {whole[0]}')
    return 0


# Layouts whose sectors sit within 1% of a revolution of the best fit
# count as tied: real drives and formatters vary that much.
FIT_TOLERANCE = 0.01


class Match(typing.NamedTuple):
    full: bool
    found: int
    expected: int
    misfit: float
    name: str


def probe(read, diskdefs):
    """Formats ranked by the tracks `read(cyl, head)` returns, and the head
    step the disk needs.

    Decoding checks each sector's ID, size and data rate, which leaves 22
    groups of formats alike in gw 1.23. The rest of gw's template tells them
    apart: index mark, interleave, skew, gaps, length and unformatted tracks.
    So formats rank by how far their sectors sit from where each writes them,
    and tracks the leaders would write differently are read."""
    from greaseweazle.codec import codec
    disks = {}
    # gw's own formats, then a definitions file's, which win a shared name.
    for source in [None, diskdefs] if diskdefs else [None]:
        for name in codec.get_all_formats('', codec.DiskDef_File(source)):
            if name.endswith('.scan'):  # scans read any layout, so prove nothing
                continue
            with contextlib.suppress(Exception):
                disks[name] = codec.get_diskdef(name, source)
    print('Trying every format gw knows...')
    tracks = {k: read(*k) for k in [(0, 0), (0, 1)]}
    decoded = {}
    ranked = rank(disks, tracks, decoded)
    if not any(m.full for m in ranked):
        return ranked, 1
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
        key = divergence(close, tracks, step, layouts)
        if key is None:
            break
        tracks[key] = read(key[0] * step, key[1])
        ranked = rank({m.name: disks[m.name] for m in leaders}, tracks, decoded)
    return filesystem(ranked, disks, tracks, read, step), step


def stepping(disk, track):
    """2 if physical cylinder 2 reads as this format's cylinder 1, not 2."""
    def at(c):
        tdef = disk.track_map.get((c, 0))
        return decode(tdef, (c, 0), track)[0] if tdef else 0
    return 2 if at(1) > at(2) else 1


def divergence(disks, tracks, step, layouts):
    """The first unread track that tells apart two of these formats that
    every track read so far leaves alike, or None."""
    def alike(a, b, key):
        return layout(a, key, layouts) == layout(b, key, layouts)
    pairs = [(a, b) for i, a in enumerate(disks) for b in disks[i + 1:]
             if all(alike(a, b, k) for k in tracks)]
    if not pairs:
        return None
    for c in range(max(d.cyls for d in disks)):
        if c * step > 83:
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
    layout. Sectors on the disk that the format does not expect count as
    expected and not found: a format that would drop them does not fit.
    A track the image lacks is blank."""
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
            raise error.Fatal('The Greaseweazle did not answer.')
        finally:
            ser.timeout = wait

    usb.Unit.__init__ = patient


def gw(args):
    from greaseweazle import cli
    steady_handshake()
    sys.argv = ['gw'] + args
    return cli.main()


def detect_like_gw(args):
    """Runs detect with its output on stderr and errors reported as gw does."""
    sys.stdout = sys.stderr
    try:
        steady_handshake()
        return detect(args)
    except Exception as e:
        print('** FATAL ERROR:\n' + str(e))
        return 1


def latest(repo=GW_REPO):
    """The tag of a repository's newest release. GitHub leaves out prereleases."""
    import requests
    try:
        reply = requests.get(f'{GITHUB_API}/repos/{repo}/releases/latest', timeout=(5, 15))
    except requests.RequestException:
        raise ValueError('Could not reach GitHub to check for a newer release.') from None
    if reply.status_code == 404:
        raise ValueError(f'{repo} has no release on GitHub.')
    reply.raise_for_status()
    return reply.json()['tag_name']


def fetch(tag, name, folder):
    """Downloads a release asset of Ferriteweazle into folder, refusing one
    whose SHA-256 is not the release's own; a zip or tarball is unpacked
    into folder/unpacked. The path of what to install."""
    import hashlib, requests, shutil, tarfile, zipfile
    base = f'{GITHUB}/{APP_REPO}/releases/download/{tag}'
    sums = requests.get(f'{base}/Ferriteweazle-{tag.lstrip("v")}-SHA256SUMS.txt', timeout=(5, 30))
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
        raise ValueError(f'gw {tag} changes its C code or its dependencies, '
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
    try:
        print(work(), flush=True)
    except Exception as e:
        sys.exit(str(e) or type(e).__name__)


if __name__ == '__main__':
    mode, args = sys.argv[1], sys.argv[2:]
    {'serve': serve,
     'run': lambda: run(lambda: gw(args)),
     'detect': lambda: run(lambda: detect_like_gw(args)),
     'latest': lambda: reported(lambda: latest(*args)),
     'fetch': lambda: reported(lambda: fetch(*args)),
     'update': lambda: reported(lambda: update(*args))}[mode]()
