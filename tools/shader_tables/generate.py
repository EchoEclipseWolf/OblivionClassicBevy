#!/usr/bin/env python3
"""Generate bs/shader_tables.rs from the C++ .inc tables and the 00B32590 table."""
import re,sys
B='/run/media/natfoth/Echo/OblivionRecomp/src/'
incs=['bs/shader_tallgrass.inc','bs/shader_imagespace_1.inc','bs/shader_hdr.inc','bs/shader_gethit.inc','bs/shader_water.inc','bs/shader_distant_lod.inc','bs/shader_water17.inc','bs/shader_lighting30.inc','bs/shader_shadow_light.inc']
def upper(n):
    n=n[1:]
    n=re.sub(r'(?<=[a-z0-9])(?=[A-Z])','_',n)
    return n.upper()
KIND={'kGuard':'Guard','kStage':'Stage','kState':'State','kVertex':'Vertex','kPixel':'Pixel','kTable':'Table','kRelease':'Release','kStages7':'Stages7','kAddress1':'Address1','kFlush':'Flush','kStageAt':'StageAt','kTableCopy':'TableCopy'}
MK={'kMapCreate':'Create','kMapAdd':'Add','kMapPredefined':'Predefined','kMapGet':'Get','kMapCopy':'Copy'}
out=['//! Generated from the C++ port\'s `src/bs/shader_*.inc` tables, which were generated from the exact stack','//! images and builder traces of the executable. Do not edit by hand.','//!','//! The definitions "@SMCLASS@", "@SPEC@" and "@EMIS@" stand for the run-time Buffer strings.','','use super::shader_classes_common::{MapBuildKind as M, PassBuildKind as K, ProgramRecord, m, o, r};','use super::shader_classes_common::{MapBuildOp, PassBuildOp};','']
def num(x):
    return x.replace('U','').replace('u','')
def conv_rec(line):
    # {"path", {{"A", "b"}, {"C", std::nullopt}}},
    res=[]
    for mt in re.finditer(r'\{"((?:[^"\\]|\\.)*)",\s*\{((?:\{[^{}]*\}(?:,\s*)?)*)\}\}',line):
        ms=[]
        for mm in re.finditer(r'\{"([^"]*)",\s*("([^"]*)"|std::nullopt|nullptr)\}',mt.group(2)):
            d='None' if not mm.group(2).startswith('"') else 'Some("%s")'%mm.group(3)
            ms.append('("%s", %s)'%(mm.group(1),d))
        res.append('    r("%s", &[%s]),'%(mt.group(1),', '.join(ms)))
    return res
for inc in incs:
    text=open(B+inc,encoding='utf-8').read().split('\n')
    cur=None
    pending=''
    for ln in text:
        s=ln.strip()
        if cur is None:
            if s.startswith('//'):
                if 'Do not edit' in s or s.startswith('// Generated') or 'gen_descriptors' in s or 'stand for the run-time' in s or 'Buffer strings' in s or 'edit by hand' in s: continue
                out.append(s)
            m_=re.match(r'(?:const|constexpr) std::array<(\w+), (\d+)> (\w+)\{\{',s)
            if m_:
                ty,n,name=m_.groups()
                rt={'ShaderProgramDescriptor':'ProgramRecord','PassBuildOp':'PassBuildOp','MapBuildOp':'MapBuildOp'}[ty]
                out.append('pub static %s: [%s; %s] = ['%(upper(name),rt,n)); cur=ty; out.append('')  ; out.pop()
                continue
            if s and not s.startswith('//'): raise SystemExit('unparsed '+inc+': '+s)
        else:
            if s=='}};':
                out.append('];'); out.append(''); cur=None; pending=''; continue
            if s.startswith('//'): out.append('    '+s); continue
            if cur=='ShaderProgramDescriptor':
                pending+=s
                if pending.endswith('},') and pending.count('{')==pending.count('}'):
                    c=conv_rec(pending)
                    if len(c)!=1: raise SystemExit('rec '+pending)
                    out+=c; pending=''
            elif cur=='PassBuildOp':
                row=[]
                for mt in re.finditer(r'\{(k\w+),\s*([^{}]*)\}',s):
                    v=[num(x.strip()) for x in mt.group(2).split(',')]
                    while len(v)<5: v.append('0')
                    row.append('o(K::%s, %s)'%(KIND[mt.group(1)],', '.join(v)))
                out.append('    '+', '.join(row)+',')
            else:
                pending+=s
                if pending.endswith('},'):
                    mt=re.match(r'\{(k\w+),(.*)\},$',pending)
                    v=[]
                    for x in re.findall(r'"[^"]*"|nullptr|[^,\s]+',mt.group(2)):
                        v.append('None' if x=='nullptr' else ('Some(%s)'%x if x.startswith('"') else num(x)))
                    out.append('    m(M::%s, %s),'%(MK[mt.group(1)],', '.join(v)))
                    pending=''
# predefined names
rt=open(B+'gamebryo/shader_runtime.cpp',encoding='utf-8').read()
i=rt.index('kPredefinedConstantNames{{'); j=rt.index('}};',i)
rows=re.findall(r'\{(\d+), "(\w+)"\}',rt[i:j])
assert len(rows)==108
out.append('// IDA: 00B32590 the 108 {id, name} predefined mappings (count 00B32584), searched in order with _stricmp by 009A47E0.')
out.append('pub static PREDEFINED_CONSTANT_NAMES: [(u32, &str); 108] = [')
for a,b in rows: out.append('    (%s, "%s"),'%(a,b))
out.append('];')
open(sys.argv[1],'w',encoding='utf-8').write('\n'.join(out)+'\n')
print(len(out))
