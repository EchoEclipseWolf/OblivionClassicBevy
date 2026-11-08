#!/usr/bin/env python3
"""The same run's lists on both sides: DataHandler lists, editor IDs, extras, cells, worlds and door nodes.

ledger_lists.py SNAPSHOT_NAME PORT_DUMP, as phase16_end, from the snapshots in ~/oblivion-capture/run-b1-snap. See
docs/original-debug.md.
"""
import sys,os,collections,re
import ledger_compare as L
from snapshot import Snapshot
s=Snapshot(os.path.expanduser('~/oblivion-capture/run-b1-snap/snapshot-'+sys.argv[1]))
P=open(os.path.expanduser(sys.argv[2])).read().splitlines()
ptr={}
for _,k,v in L.walk_map(s,L.FORMS_MAP): ptr[v]=k
fid=lambda p: ptr.get(p,0)
def lst(node):
    out=[]
    while node and len(out)<200000:
        item,nxt=s.u32(node),s.u32(node+4)
        if item is None: break
        if item: out.append(item)
        node=nxt
    return out
def hexs(v): return ','.join('%X'%fid(x) for x in v)
# LIST
dh=s.u32(0xB33A98)
pl={l.split(' ',2)[1]:(l.split(' ',2)+[''])[2] for l in P if l.startswith('LIST ')}
bad=0
for off,ids in pl.items():
    o=int(off,16)
    if o==0xBC:
        head=s.u32(dh+0xBC)+4
    else:
        head=dh+o
    orig=hexs(lst(head))
    if orig!=ids:
        bad+=1; a,b=orig.split(','),ids.split(',')
        i=next((i for i,(x,y) in enumerate(zip(a,b)) if x!=y),min(len(a),len(b)))
        print(' LIST %s: original %d port %d, first diff at %d: %s / %s'%(off,len(a),len(b),i,a[i:i+3],b[i:i+3]))
print('LIST',len(pl),'compared, differ',bad)
# EDID
oe=[]
sz,bk=s.u32(0xB06164+4),s.u32(0xB06164+8)
for i in range(sz):
    n=s.u32(bk+i*4)
    while n:
        k=s.u32(n+4);v=s.u32(n+8)
        t=s.text(k,120).replace(' ','_')
        oe.append((t,fid(v)))
        n=s.u32(n)
pe=[(l.split(' ')[1],int(l.split(' ')[2],16)) for l in P if l.startswith('EDID ')]
so,sp=collections.Counter(oe),collections.Counter(pe)
print('EDID original',len(oe),'port',len(pe),'only original',sum((so-sp).values()),'only port',sum((sp-so).values()))
print('  only port',list((sp-so).items())[:12])
print('  only original',[(repr(a)[:26],'%X'%b) for (a,b),n in list((so-sp).items())[:40]])
# order
same=[x for x in oe if x in sp]; samep=[x for x in pe if x in so]
print('  same order over shared:',same==samep)
for i,(a,b) in enumerate(zip(same,samep)):
    if a!=b:
        print('  first order diff at',i,a,b,'next',same[i+1:i+3],samep[i+1:i+3]); break
# XTRA
def extras(owner_list_head):
    out=[];n=owner_list_head
    while n and len(out)<64:
        out.append(n); n=s.u32(n+8)
    return out
def w(a): return s.u32(a)
def value(n):
    t=s.u8(n+4)
    return t,n
PX=collections.defaultdict(list)
for l in P:
    if l.startswith('XTRA '):
        p=l.split(' ',3); PX[int(p[1],16)].append((int(p[2],16),p[3] if len(p)>3 else ''))
cat=collections.Counter(); samples={}
def fmt(t,n,val):
    a=w(n+0xC)
    if val.startswith('R'):
        if t==0x31: return 'R%X'%fid(w(a+4))
        if t==0x32: return 'R%X'%fid(w(a))
        return 'R%X'%fid(a)
    if t==0x31:
        b=s.read(a,12); return b.hex().upper() if b else '?'
    if t==0x32:
        return ','.join('%X'%w(a+i*4) for i in range(1,7))
    if t==0x14:
        return ','.join('%X'%w(n+0xC+i*4) for i in range(6))
    if t==0x36:
        return '%X,%X'%(a,s.u8(n+0x10))
    if t==0x3F:
        return '%X,%X'%(fid(a) or a,s.u8(n+0x10))
    if t==0x18:
        return ','.join('%X'%w(n+0xC+i*4) for i in range(3))
    if t==0x1A:
        out=[]; h=w(a)
        if not h: return ''
        node=w(h)
        while True:
            break
        # head list inline: entry list ptr at block+0
        node=a and w(a)
        ents=[]
        c=0
        while node and c<4000:
            e,nx=w(node),w(node+4)
            if e:
                ls=w(e); cnt=0
                nn=ls
                while nn and cnt<4000:
                    it,nnx=w(nn),w(nn+4)
                    if it: cnt+=1
                    nn=nnx
                ents.append('%X:%d:%d'%(fid(w(e+8)),(w(e+4) if w(e+4)<0x80000000 else w(e+4)-(1<<32)),cnt))
            node=nx;c+=1
        return ','.join(ents)
    if t in (0x40,0x23):
        return ','.join('%X'%fid(x) for x in (lst(a) if t==0x23 else lst(n+0xC)))
    if val=='?': return '?'
    # dword/word/byte/float by width of the port's text
    return None
tot=0
for _,k,v in L.walk_map(s,L.FORMS_MAP):
    kind=s.u8(v+4)
    if kind in (0x31,0x32,0x33): h=w(v+0x48)
    elif kind==0x30: h=w(v+0x28+4)
    else: continue
    px=PX.get(k,[])
    ox=extras(h)
    if len(ox)!=len(px): cat[('count',kind)]+=1; samples.setdefault(('count',kind),(k,[s.u8(n+4) for n in ox],[p[0] for p in px])); continue
    for n,(pt,pv) in zip(ox,px):
        t=s.u8(n+4); tot+=1
        if t!=pt: cat[('type',t,pt)]+=1; continue
        want=fmt(t,n,pv)
        if want is None:
            raw=[w(n+0xC), s.u8(n+0xC), (w(n+0xC) or 0)&0xFFFF]
            ok=int(pv,16) in raw if pv and pv!='?' else False
            if pv=='?': cat[('unmapped',t)]+=1
            elif not ok: cat[('value',t)]+=1; samples.setdefault(('value',t),(hex(k),pv,hex(w(n+0xC))))
        elif want!='?' and want!=pv:
            cat[('value',t)]+=1; samples.setdefault(('value',t),(hex(k),pv[:80],want[:80]))
print('XTRA compared',tot,sorted(cat.items(),key=lambda x:-x[1])[:24])
for kk,vv in list(samples.items())[:18]: print('  ',kk,vv)
# CELL
PC={int(l.split(' ')[1],16):l.split(' ') for l in P if l.startswith('CELL ')}
c=collections.Counter(); smp={}
for _,k,v in L.walk_map(s,L.FORMS_MAP):
    if s.u8(v+4)!=0x30: continue
    p=PC.get(k)
    if not p: c['missing']+=1; continue
    objs=hexs(lst(v+0x48))
    fl='%X'%s.u8(v+0x24); f25='%X'%s.u8(v+0x25); pl='%X'%s.u8(v+0x26)
    land='%X'%fid(w(v+0x40)); pg='%X'%fid(w(v+0x44)); wd='%X'%fid(w(v+0x50))
    d=w(v+0x3C); interior=s.u8(v+0x24)&1
    data=','.join('%X'%w(d+i*4) for i in range(10 if interior else 2)) if d else '-'
    pdata=p[7] if not interior or p[7]=='-' else p[7]
    if p[7]!='-' and not interior: pdata=','.join(p[7].split(',')[:2])
    names=['flags','f25','level','land','grid','world','data','objects']
    vals=[fl,f25,pl,land,pg,wd,data,objs]
    ports=[p[2],p[3],p[4],p[5],p[6],p[7+0] if False else p[7+0],pdata,(p[8] if len(p)>8 else '')]
    ports[5]=p[7] if False else p[7-0] if False else p[7]
    # order of fields: id, flags, 25, level, land, grid, world, data, objects
    ports=[p[2],p[3],p[4],p[5],p[6],p[7],None,None]
    ports[6]=p[8]; ports[7]=p[9] if len(p)>9 else ''
    if not interior and ports[6]!='-': ports[6]=','.join(ports[6].split(',')[:2])
    for nm,a,b in zip(names,vals,ports):
        if a!=b:
            c[nm]+=1; smp.setdefault(nm,('%X'%k,a[:60],b[:60]))
print('CELL',len(PC),dict(c))
for kk,vv in smp.items(): print('  ',kk,vv)
# WRLD
PW={int(l.split(' ')[1],16):l.split(' ') for l in P if l.startswith('WRLD ')}
c=collections.Counter();smp={}
for _,k,v in L.walk_map(s,L.FORMS_MAP):
    if s.u8(v+4)!=0x35 and k not in PW: continue
    p=PW.get(k)
    if not p: continue
    vals=['%X'%fid(w(v+0x34)),'%X'%fid(w(v+0x54)),'%X'%w(w(v+0x30)+0xC) if False else None]
    pers=vals[0]; road=vals[1]
    if pers!=p[2]: c['persistent']+=1
    if road!=p[3]: c['road']+=1
    un=hexs(lst(v+0x74))
    if un!=(p[5] if len(p)>5 else ''): c['unsorted']+=1; smp.setdefault('unsorted',('%X'%k,un[:60],(p[5] if len(p)>5 else '')[:60]))
    # grid map at +0x64: inline header
    grid=[]
    for i in range(w(v+0x64+4) or 0):
        n=w(w(v+0x64+8)+i*4)
        while n:
            key,val=w(n+4),w(n+8)
            grid.append('%X=%s'%(key,'+'.join('%X'%fid(x) for x in lst(val))))
            n=w(n)
    grid.sort()
    pg=p[6] if len(p)>6 else ''
    if ';'.join(grid)!=pg:
        c['grid']+=1; a=';'.join(grid);smp.setdefault('grid',('%X'%k,len(grid),len(pg.split(';')),a[:70],pg[:70]))
print('WRLD',len(PW),dict(c))
for kk,vv in smp.items(): print('  ',kk,vv)
# DOOR: listHead_ prepends, so its reverse is build order. 00B3BE00's keys are every node's ends.
door_nodes=lst(0xB3BE18)[::-1]
od=[tuple(fid(s.u32(p+o)) for o in (4,8,12,16)) for p in door_nodes]
pd=[tuple(int(x,16) for x in l.split()[1:]) for l in P if l.startswith('DOOR ')]
print('DOOR original',len(od),'port',len(pd),'differ',sum(1 for a,b in zip(od,pd) if a!=b))
okeys={fid(k) for _,k,_ in L.walk_map(s,s.u32(0xB3BE00))}
pkeys={v for n in pd for v in (n[1],n[3])}
print('DOOR keys original',len(okeys),'port',len(pkeys),'differ',len(okeys^pkeys))
