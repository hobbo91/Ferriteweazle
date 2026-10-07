"""Takes the disk's data out of recorded bridge reports, for the repository:
each block of bytes that is not gw's filler or one byte over and over
becomes a count of the same length, each byte one more than the last, so
that it still reads as data. The flux counts and everything else stay.

    python scrub.py FILE...
"""
import json, sys

PREFIX = '@ferriteweazle '
FILLER = b'-=[BAD SECTOR]=-'


def kept(b):
    """Bytes that are no disk's own: gw's filler, one byte, or a count."""
    return (len(set(b)) <= 1
            or all(b[i:i + 16] == FILLER[:len(b[i:i + 16])] for i in range(0, len(b), 16))
            or all((b[i + 1] - b[i]) % 256 == 1 for i in range(len(b) - 1)))


def scrubbed(hexs):
    b = bytes.fromhex(hexs)
    return hexs if kept(b) else bytes(i % 256 for i in range(len(b))).hex()


def walk(v):
    """Scrubs `v` in place; whether it changed anything."""
    changed = False
    if isinstance(v, dict):
        for k, x in v.items():
            if k == 'bytes' and isinstance(x, str):
                v[k] = scrubbed(x)
                changed |= v[k] != x
            elif k == 'data' and isinstance(x, dict):
                v[k] = {n: scrubbed(h) for n, h in x.items()}
                changed |= v[k] != x
            else:
                changed |= walk(x)
    elif isinstance(v, list):
        for x in v:
            changed |= walk(x)
    return changed


for path in sys.argv[1:]:
    lines = open(path).read().split('\n')
    for i, line in enumerate(lines):
        if line.startswith(PREFIX):
            kind, _, body = line[len(PREFIX):].partition(' ')
            if body.startswith('{'):
                report = json.loads(body)
                if walk(report):
                    lines[i] = PREFIX + kind + ' ' + json.dumps(report, separators=(',', ':'))
    open(path, 'w').write('\n'.join(lines))
