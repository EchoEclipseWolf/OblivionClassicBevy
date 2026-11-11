"""Minimal D3D9 shader bytecode disassembler (SM 2/3 pixel and vertex shaders) for reading Oblivion's shader packages."""
import struct
import sys

OPS = {
    0: 'nop', 1: 'mov', 2: 'add', 3: 'sub', 4: 'mad', 5: 'mul', 6: 'rcp', 7: 'rsq', 8: 'dp3', 9: 'dp4', 10: 'min',
    11: 'max', 12: 'slt', 13: 'sge', 14: 'exp', 15: 'log', 16: 'lit', 17: 'dst', 18: 'lrp', 19: 'frc', 20: 'm4x4',
    21: 'm4x3', 22: 'm3x4', 23: 'm3x3', 24: 'm3x2', 25: 'call', 26: 'callnz', 27: 'loop', 28: 'ret', 29: 'endloop',
    30: 'label', 31: 'dcl', 32: 'pow', 33: 'crs', 34: 'sgn', 35: 'abs', 36: 'nrm', 37: 'sincos', 38: 'rep',
    39: 'endrep', 40: 'if', 41: 'ifc', 42: 'else', 43: 'endif', 44: 'break', 45: 'breakc', 46: 'mova', 47: 'defb',
    48: 'defi', 64: 'texcoord', 65: 'texkill', 66: 'texld', 67: 'texbem', 81: 'def', 82: 'texreg2rgb', 88: 'cmp',
    89: 'bem', 90: 'dp2add', 91: 'dsx', 92: 'dsy', 93: 'texldd', 94: 'setp', 95: 'texldl', 96: 'breakp',
}
REGT = {0: 'r', 1: 'v', 2: 'c', 3: 't', 4: 'rasout', 5: 'attrout', 6: 'o', 7: 'input', 8: 'oC', 9: 'oDepth',
        10: 's', 14: 'b', 15: 'aL', 17: 'misc', 18: 'label', 19: 'p'}


def regtype(tok):
    return ((tok >> 28) & 7) | ((tok >> 8) & 0x18)


def dst(tok):
    t = regtype(tok)
    s = '%s%d' % (REGT.get(t, '?%d' % t), tok & 0x7FF)
    m = (tok >> 16) & 0xF
    mod = (tok >> 20) & 0xF
    if mod & 1:
        s += '_sat'
    if m != 0xF:
        s += '.' + ''.join('xyzw'[i] for i in range(4) if m >> i & 1)
    return s


def src(tok):
    t = regtype(tok)
    s = '%s%d' % (REGT.get(t, '?%d' % t), tok & 0x7FF)
    sw = (tok >> 16) & 0xFF
    comps = ''.join('xyzw'[(sw >> (2 * i)) & 3] for i in range(4))
    if comps != 'xyzw':
        s += '.' + (comps[0] if len(set(comps)) == 1 else comps)
    mod = (tok >> 24) & 0xF
    if mod == 1:
        s = '-' + s
    elif mod == 11:
        s = 'abs(' + s + ')'
    elif mod == 12:
        s = '-abs(' + s + ')'
    elif mod:
        s = 'mod%d(%s)' % (mod, s)
    return s


def ctab(words):
    raw = struct.pack('<%dI' % len(words), *words)
    if raw[:4] != b'CTAB':
        return []
    c = raw[4:]
    _, creator, ver, n, cinfo, flags, target = struct.unpack_from('<7I', c, 0)
    out = []
    for i in range(n):
        name, regset, reg, count, _, typeinfo, default = struct.unpack_from('<IHHHHII', c, cinfo + 20 * i)
        nm = c[name:c.index(b'\0', name)].decode()
        out.append('%s %s%d x%d' % (nm, ['b', 'i', 'c', 's'][regset], reg, count))
    return out


def disasm(data):
    words = struct.unpack('<%dI' % (len(data) // 4), data[:len(data) // 4 * 4])
    ver = words[0]
    lines = ['%s_%d_%d' % ('ps' if ver >> 16 == 0xFFFF else 'vs', (ver >> 8) & 0xFF, ver & 0xFF)]
    i = 1
    while i < len(words):
        tok = words[i]
        op = tok & 0xFFFF
        if tok == 0x0000FFFF:
            break
        if op == 0xFFFE:
            n = (tok >> 16) & 0x7FFF
            for c in ctab(list(words[i + 1:i + 1 + n])):
                lines.append('// ' + c)
            i += 1 + n
            continue
        length = (tok >> 24) & 0xF
        if (ver >> 8) & 0xFF < 2:
            # Shader model 1 leaves the length bits clear: def takes a register and four floats, and any other
            # instruction's parameter tokens are those with bit 31 set.
            if op == 81:
                length = 5
            else:
                length = 0
                while i + 1 + length < len(words) and words[i + 1 + length] >> 31:
                    length += 1
        args = list(words[i + 1:i + 1 + length])
        name = OPS.get(op, 'op%d' % op)
        if op == 81:
            f = [struct.unpack('<f', struct.pack('<I', a))[0] for a in args[1:]]
            lines.append('def %s, %s' % (dst(args[0]), ', '.join('%g' % x for x in f)))
        elif op == 31:
            lines.append('dcl_%08X %s' % (args[0], dst(args[1])))
        elif args:
            ctrl = (tok >> 16) & 0xFF
            suffix = '_pp' if (args[0] >> 20) & 2 else ''
            lines.append('%s%s %s' % (name, suffix, ', '.join([dst(args[0])] + [src(a) for a in args[1:]])))
        else:
            lines.append(name)
        i += 1 + length
    return '\n'.join(lines)


def package(path):
    d = open(path, 'rb').read()
    _, count, _ = struct.unpack_from('<III', d, 0)
    o = 12
    out = {}
    for _ in range(count):
        name = d[o:o + 0x100].split(b'\0')[0].decode()
        ln = struct.unpack_from('<I', d, o + 0x100)[0]
        out[name] = d[o + 0x104:o + 0x104 + ln]
        o += 0x104 + ln
    return out


if __name__ == '__main__':
    pkg = package(sys.argv[1])
    for name in sys.argv[2:]:
        print('=====', name)
        print(disasm(pkg[name]))
