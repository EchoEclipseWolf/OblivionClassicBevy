# Capture set A: one default-path run of the original, from WinMain to an idle MainMenu.
# Runs inside IDA (idat through the ida MCP); see docs/original-debug.md. Set ida_dbg.CAP_OUT (a folder),
# CAP_AUDIT (docs/boundary-audit.md), CAP_DATA_HANDLER and CAP_SETS (any of audit, edid, doors), exec
# this file, call CAPF['start'](), start the process, then CAPF['pump']() until the checkpoints are done.
# Every stop costs about 100 ms with 500 breakpoints armed, so frequent probes get their own run.
#
# What it records: every _srand with its seed and caller, _holdrand per thread at the checkpoints, the
# first hit of each stop and missing address in the audit, hits at 00680A00 and its gates, every
# insert into the editor ID map with the count before and after, a walk of that map and of
# DataHandler +0xBC at 004067CC, and the control word at 0074FF37 by thread.
#
# The set `window` adds first-hit call coverage over a window: each function reached in it, by any thread,
# in hit order, and the targets of its indirect calls. ida_dbg.CAP_WINDOW picks the window, Phase 18's by
# default; its `cps` are one-shot checkpoints armed only inside it. CAP_SNAPS, when set, names the
# checkpoints that take a snapshot.
import ida_dbg, ida_funcs, ida_idd, idc, ida_bytes, ida_segment, idautils, time, json, os

P = ida_dbg.CAP = dict(recs=[], seq=0, probes={}, hits={}, ptd={}, tebs={}, done=False, events=0,
                       started=time.time(), last_cp=None, pending_insert=None)
OUT = ida_dbg.CAP_OUT
os.makedirs(OUT, exist_ok=True)

SRAND_AFTER_PTD = 0x9859D5   # _srand: eax = ptd, [esp+4] = seed
RAND = 0x9859DD
HOLDRAND = 0x14
EDID_MAP = 0xB06164
EDID_INSERT = 0x44F2D9
EDID_AFTER = 0x44F2DE
# Inside 00680090: past the type, prison and teleport gates; past 0067F5D0; 00680A00 answered non-null.
DOORS = {0x6800D3: 'door_gated', 0x6800E0: 'door_680A00', 0x6800EE: 'door_built'}
CHECKPOINTS = {0x4067CC: 'load_end', 0x40EBB9: 'world_setup_end', 0x40EE7A: 'phase16_start',
               0x40EF54: 'phase16_end', 0x40EFF3: 'phase17_end', 0x40F04A: 'phase18_spell',
               0x40F055: 'phase18_saved', 0x40F069: 'phase18_end', 0x40F07F: 'phase19_propagated',
               0x40F08D: 'phase19_end', 0x5B6093: 'menu_closed', 0x40F092: 'phase20_end', 0x40F0E0: 'phase21_end', 0x40F26A: 'idle_frame_start', 0x40F275: 'idle_frame_end',
               0x40F16A: 'first_idle'}
# Registers Phase 18 reads once: the site's register and what it means.
WINDOW_REGS = {0x40EFF9: ('EBP', 'arg of 005E02E0'), 0x40F011: ('EDI', 'spell'), 0x40F019: ('EAX', 'school'),
               0x40F045: ('EDI', 'item'), 0x66474E: ('EAX', 'effect'), 0x6647DE: ('ESP', 'icon path'),
               0x6632CB: ('EAX', 'save id'), 0x6632D9: ('EAX', 'save size'), 0x6632F7: ('EAX', 'buffer')}
# Phase 19 reads the predicate's answer once.
WINDOW_18 = dict(start='phase17_end', end='phase18_end', range=(0x40EFF3, 0x40F069), regs=WINDOW_REGS, cps={})
WINDOW_19 = dict(start='phase18_end', end='phase19_end', range=(0x40F069, 0x40F08D),
                 regs={0x40F06E: ('EAX', 'parent cell')},
                 cps={0x5ADBC0: 'loading_dtor', 0x404C60: 'purge_start', 0x43BEB0: 'purge_models',
                      0x43FC20: 'purge_tes'})
WINDOW_20 = dict(start='phase19_end', end='phase20_end', range=(0x40F08D, 0x40F092), regs={},
                 cps={0x5B6102: 'menu_accumulator', 0x5B6121: 'menu_xml', 0x5B5FB3: 'menu_ctor_movie',
                      0x5B6126: 'menu_xml_done', 0x5B6342: 'menu_saves', 0x5B6485: 'menu_enable'})
WINDOW_21 = dict(start='phase20_end', end='phase21_end', range=(0x40F092, 0x40F0E0), regs={},
                 cps={0x40F097: 'intro_thread_done', 0x40F0A4: 'target_done', 0x40F0AF: 'title_music_done',
                      0x40F0C3: 'logo_done', 0x40F0C8: 'intro_done'})
WINDOW_22 = dict(start='phase21_end', end='idle_frame_end', range=(0x40F0E0, 0x40F27A), regs={}, cps={})
# Heap, CRT, RTTI and logging: logged when reached, never expanded.
COVER_LEAF = (0x980000, 0xA00000)
X87 = 0x74FF37
DATA_HANDLER = None


def dw(ea):
    b = idc.read_dbg_memory(ea, 4) if ea else None
    return int.from_bytes(b, 'little') if b and len(b) == 4 else 0


def text(ea, n=80):
    if not ea:
        return ''
    b = idc.read_dbg_memory(ea, n) or b''
    b = b.split(b'\0')[0]
    return b.decode('latin-1').replace(' ', '_').replace('\n', '_')


def R(n):
    return ida_dbg.get_reg_val(n) & 0xFFFFFFFF


def log(s):
    P['seq'] += 1
    P['recs'].append('%d %s' % (P['seq'], s))
    if len(P['recs']) > 500:
        flush()


def flush():
    with open(OUT + '/records.txt', 'a') as f:
        f.write(''.join(r + '\n' for r in P['recs']))
    P['recs'] = []


def arm(ea, kind):
    # A breakpoint inside an instruction corrupts it: 0040EBB8 sat in a call's target and crashed the game.
    flags = ida_bytes.get_flags(ea)
    if not (ida_bytes.is_head(flags) and ida_bytes.is_code(flags)):
        raise ValueError('not an instruction start: %X' % ea)
    P['probes'][ea] = kind
    if not ida_dbg.exist_bpt(ea):
        ida_dbg.add_bpt(ea, 0, ida_idd.BPT_SOFT)


def disarm(ea):
    P['probes'].pop(ea, None)
    if ida_dbg.exist_bpt(ea):
        ida_dbg.del_bpt(ea)


def tid():
    d = ida_dbg.get_current_thread()
    return d


def holdrands():
    return ' '.join('%X:%08X' % (t, dw(p + HOLDRAND)) for t, p in sorted(P['ptd'].items()))


def walk_edid_map():
    size, buckets, count = dw(EDID_MAP + 4), dw(EDID_MAP + 8), dw(EDID_MAP + 0xC)
    log('EDMAP size=%X buckets=%X count=%X' % (size, buckets, count))
    table = idc.read_dbg_memory(buckets, size * 4) or b''
    n = 0
    with open(OUT + '/edid-map.txt', 'w') as f:
        for i in range(size):
            node = int.from_bytes(table[i * 4:i * 4 + 4], 'little')
            pos = 0
            while node and pos < 4096:
                head = idc.read_dbg_memory(node, 12) or b'\0' * 12
                nxt = int.from_bytes(head[0:4], 'little')
                key = int.from_bytes(head[4:8], 'little')
                val = int.from_bytes(head[8:12], 'little')
                f.write('%d %d node=%X key=%X val=%X id=%X type=%X text=%s\n' % (
                    i, pos, node, key, val, dw(val + 0xC), (dw(val + 4) & 0xFF), text(key)))
                node = nxt
                pos += 1
                n += 1
    log('EDMAP walked=%d' % n)


def walk_regions():
    dh = DATA_HANDLER and dw(DATA_HANDLER)
    if not dh:
        log('REGIONS no data handler')
        return
    head = dw(dh + 0xBC)
    raw = idc.read_dbg_memory(head, 0x20) if head else b''
    log('REGIONS dh=%X bc=%X raw=%s' % (dh, head, raw.hex() if raw else 'null'))
    node = head + 4 if head else 0
    ids = []
    while node and len(ids) < 2000:
        item, nxt = dw(node), dw(node + 4)
        if item:
            ids.append('%X' % dw(item + 0xC))
        node = nxt
    log('REGIONS walked=%d ids=%s' % (len(ids), ','.join(ids)))


def snapshot(name):
    # Every writable region and the exe's image, so later questions are read from a file, not a run.
    folder = '%s/snapshot-%s' % (OUT, name)
    os.makedirs(folder, exist_ok=True)
    t = time.time()
    index = []
    total = 0
    ea = 0
    ida_dbg.refresh_debugger_memory()
    for start in idautils.Segments():
        m = ida_segment.getseg(start)
        if m.start_ea >= 0x80000000:
            continue
        if not (m.perm & 2) and not (0x400000 <= m.start_ea < 0xC00000):
            continue
        name_ = '%08X.bin' % m.start_ea
        size = 0
        with open(folder + '/' + name_, 'wb') as f:
            a = m.start_ea
            while a < m.end_ea:
                n = min(0x100000, m.end_ea - a)
                b = idc.read_dbg_memory(a, n)
                if not b:
                    b = b'\0' * n
                f.write(b)
                a += n
                size += n
        index.append(dict(start=m.start_ea, end=m.end_ea, perm=m.perm, name=ida_segment.get_segm_name(m)))
        total += size
    threads = []
    here = ida_dbg.get_current_thread()
    tebs = {}
    for page in range(0x10000, 0x1000000, 0x1000):
        if dw(page + 0x18) == page:
            tebs[dw(page + 0x24)] = page
    for i in range(ida_dbg.get_thread_qty()):
        t_ = ida_dbg.getn_thread(i)
        ida_dbg.select_thread(t_)
        threads.append(dict(tid=t_, teb=tebs.get(t_, 0), eip=R('EIP'), esp=R('ESP'),
                            ctrl=ida_dbg.get_reg_val('CTRL') & 0xFFFF))
    ida_dbg.select_thread(here)
    with open(folder + '/index.json', 'w') as f:
        json.dump(dict(regions=index, threads=threads, ptd=P['ptd'], main=here, stopped_at=R('EIP')), f, indent=0)
    log('SNAP %s regions=%d bytes=%d t=%.0f threads=%s' % (name, len(index), total, time.time() - t, threads))


def checkpoint(name):
    log('CP %s tid=%X count_edid=%X ctrl=%X holdrand %s' % (
        name, tid(), dw(EDID_MAP + 0xC), ida_dbg.get_reg_val('CTRL') & 0xFFFF, holdrands()))
    if name == 'load_end':
        walk_edid_map()
        walk_regions()
    window = getattr(ida_dbg, 'CAP_WINDOW', WINDOW_18)
    if name == window['start'] and 'window' in ida_dbg.CAP_SETS:
        cover_range(*window['range'])
        for a in window['regs']:
            arm(a, 'winreg')
    if name == window['start'] and 'snapshot' in ida_dbg.CAP_SETS:
        for a, cp in window['cps'].items():
            arm(a, 'cp:' + cp)
    if name == window['end']:
        for a, kind in list(P['probes'].items()):
            if kind in ('cover', 'icall', 'winreg') or kind[3:] in window['cps'].values():
                disarm(a)
    if 'snapshot' in ida_dbg.CAP_SETS and name in getattr(ida_dbg, 'CAP_SNAPS', {name}):
        snapshot(name)
    flush()


def cover_range(start, end):
    ea = start
    while ea < end:
        if idc.print_insn_mnem(ea) == 'call':
            if idc.get_operand_type(ea, 0) in (idc.o_near, idc.o_far):
                target = idc.get_operand_value(ea, 0)
                if target not in P['probes'] and target not in P.setdefault('covered', set()):
                    arm(target, 'cover')
            elif ea not in P['probes']:
                arm(ea, 'icall')
        elif idc.print_insn_mnem(ea) == 'jmp' and idc.get_operand_type(ea, 0) in (idc.o_near, idc.o_far):
            # A tail call: j_Actor_SaveGame is one jump.
            target = idc.get_operand_value(ea, 0)
            if not start <= target < end and ida_funcs.get_func(target) and ida_funcs.get_func(target).start_ea == target:
                if target not in P['probes'] and target not in P.setdefault('covered', set()):
                    arm(target, 'cover')
        ea = idc.next_head(ea, end)


def cover_hit(ea, esp):
    P.setdefault('covered', set()).add(ea)
    log('COVER %X %s tid=%X ret=%X' % (ea, idc.get_name(ea), tid(), dw(esp)))
    disarm(ea)
    f = ida_funcs.get_func(ea)
    if f and not COVER_LEAF[0] <= ea < COVER_LEAF[1]:
        for start, end in idautils.Chunks(f.start_ea):
            cover_range(start, end)


def icall_target(ea):
    kind = idc.get_operand_type(ea, 0)
    if kind == idc.o_reg:
        return R(idc.print_operand(ea, 0).upper())
    if kind == idc.o_mem:
        return dw(idc.get_operand_value(ea, 0))
    if kind in (idc.o_phrase, idc.o_displ):
        text_ = idc.print_operand(ea, 0)
        base = text_[text_.index('[') + 1:text_.index('[') + 4].upper()
        return dw(R(base) + (idc.get_operand_value(ea, 0) if kind == idc.o_displ else 0))
    return 0


def icall_hit(ea):
    try:
        target = icall_target(ea)
    except Exception:
        # An operand the parse above misreads: logged, so the gap shows.
        log('ICALLFAIL site=%X %s' % (ea, idc.generate_disasm_line(ea, 0)))
        return
    seen = P.setdefault('icalls', {}).setdefault(ea, set())
    if target not in seen:
        seen.add(target)
        log('ICALL site=%X target=%X %s tid=%X this=%X' % (ea, target, idc.get_name(target), tid(), R('ECX')))
        # A DLL's target, Wine's here, is logged and not armed.
        if 0x401000 <= target < 0x9F0000 and target not in P['probes'] and target not in P.setdefault('covered', set()):
            arm(target, 'cover')
    if P['hits'][ea] >= 32:
        disarm(ea)


def winreg_hit(ea, esp):
    reg, meaning = getattr(ida_dbg, 'CAP_WINDOW', WINDOW_18)['regs'][ea]
    if reg == 'ESP':
        log('REG %X %s=%s' % (ea, meaning, text(dw(esp), 260)))
    else:
        log('REG %X %s=%X' % (ea, meaning, R(reg)))


def handle(ea):
    kind = P['probes'].get(ea)
    esp = R('ESP')
    P['hits'][ea] = P['hits'].get(ea, 0) + 1
    if kind == 'srand':
        t = tid()
        ptd = R('EAX')
        P['ptd'][t] = ptd
        log('SRAND tid=%X ptd=%X seed=%08X ret=%X before=%08X' % (
            t, ptd, dw(esp + 4), dw(esp), dw(ptd + HOLDRAND)))
    elif kind == 'rand_ptd':
        # One hit per thread at _rand's second instruction, to learn a thread's ptd without a seed.
        t = tid()
        if t not in P['ptd']:
            P['ptd'][t] = R('EAX')
            log('PTD tid=%X ptd=%X holdrand=%08X ret=%X' % (t, R('EAX'), dw(R('EAX') + HOLDRAND), dw(esp)))
        elif P['hits'][ea] > 64:
            disarm(ea)
    elif kind == 'edid_insert':
        P['pending_insert'] = dw(EDID_MAP + 0xC)
        key, val = dw(esp), dw(esp + 4)
        log('EDI key=%X id=%X type=%X before=%X text=%s' % (
            key, dw(val + 0xC), dw(val + 4) & 0xFF, P['pending_insert'], text(key)))
    elif kind == 'edid_after':
        log('EDA after=%X' % dw(EDID_MAP + 0xC))
    elif kind and kind.startswith('door_'):
        pass
    elif kind == 'x87':
        log('X87 tid=%X ctrl=%X stat=%X self=%X f10=%X rand=%X' % (
            tid(), ida_dbg.get_reg_val('CTRL') & 0xFFFF, ida_dbg.get_reg_val('STAT') & 0xFFFF, R('EDI'),
            dw(R('EDI') + 0x10), R('EAX')))
        if P['hits'][ea] >= 12:
            disarm(ea)
    elif kind and kind.startswith('cp:'):
        name = kind[3:]
        checkpoint(name)
        disarm(ea)
    elif kind == 'cover':
        cover_hit(ea, esp)
    elif kind == 'icall':
        icall_hit(ea)
    elif kind == 'winreg':
        winreg_hit(ea, esp)
    elif kind == 'audit':
        log('REACHED %X tid=%X ret=%X eax=%X ecx=%X' % (ea, tid(), dw(esp), R('EAX'), R('ECX')))
        disarm(ea)


def audit_addresses():
    out = []
    for line in open(ida_dbg.CAP_AUDIT):
        c = line.split('|')
        if len(c) > 4 and c[1].strip().startswith('`') and c[3].strip() in ('stop', 'missing'):
            a = int(c[1].strip().strip('`'), 16)
            if ida_bytes.is_code(ida_bytes.get_flags(a)):
                out.append(a)
    return sorted(set(out))


def start():
    global DATA_HANDLER
    DATA_HANDLER = ida_dbg.CAP_DATA_HANDLER
    arm(SRAND_AFTER_PTD, 'srand')
    arm(RAND + 5, 'rand_ptd')
    sets = ida_dbg.CAP_SETS
    if 'edid' in sets:
        arm(EDID_INSERT, 'edid_insert')
    if 'doors' in sets:
        for a, n in DOORS.items():
            arm(a, n)
    arm(X87, 'x87')
    for a, n in CHECKPOINTS.items():
        arm(a, 'cp:' + n)
    used = 0
    if 'audit' in sets:
        audit = audit_addresses()
        skip = getattr(ida_dbg, 'CAP_SKIP', set())
        for a in audit:
            if a not in P['probes'] and a not in skip:
                arm(a, 'audit')
                used += 1
    log('ARMED sets=%s audit=%d total=%d' % (sorted(sets), used, len(P['probes'])))
    flush()
    return len(P['probes'])


def pump(seconds=100):
    end = time.time() + seconds
    while time.time() < end:
        st = ida_dbg.get_process_state()
        if st == 0:
            P['done'] = True
            break
        if st == -1:
            ea = R('EIP')
            if ea in P['probes']:
                handle(ea)
            ida_dbg.continue_process()
        ev = ida_dbg.wait_for_next_event(ida_dbg.WFNE_SUSP | ida_dbg.WFNE_CONT, 2)
        if ev > 0:
            P['events'] += 1
    flush()
    return 'events=%d t=%.0f done=%s hits=%d cps_left=%s' % (
        P['events'], time.time() - P['started'], P['done'], len(P['hits']),
        [k[3:] for k in P['probes'].values() if k.startswith('cp:')])


def finish():
    """Remove the probes, write the counts and close the game."""
    counts = {'%X' % a: n for a, n in sorted(P['hits'].items())}
    log('COUNTS ' + json.dumps(counts))
    log('HOLDRAND ' + holdrands())
    flush()
    for a in list(P['probes']):
        disarm(a)
    r = ida_dbg.exit_process()
    ev = ida_dbg.wait_for_next_event(ida_dbg.WFNE_ANY, 15)
    return 'exit=%s ev=%s state=%d' % (r, ev, ida_dbg.get_process_state())


ida_dbg.CAP_WINDOWS = dict(phase18=WINDOW_18, phase19=WINDOW_19, phase20=WINDOW_20, phase21=WINDOW_21, phase22=WINDOW_22)
ida_dbg.CAPF = dict(start=start, pump=pump, finish=finish, flush=flush)
