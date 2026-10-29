//! Lossless v1 QASM comment metadata. Angles are round-trip decimal strings.
use crate::{ir::*, json::Value as V, Error, Result};
fn a(xs: impl IntoIterator<Item=V>) -> V { V::Array(xs.into_iter().collect()) }
fn s(x: impl ToString) -> V { V::Str(x.to_string()) }
fn n(x: u32) -> V { V::Int(x.to_string()) }
fn list(v: &V) -> Result<&[V]> { v.as_array().ok_or_else(|| Error("QASM metadata: expected array".into())) }
fn st(v: &V) -> Result<String> { v.as_str().map(str::to_string).ok_or_else(|| Error("QASM metadata: expected string".into())) }
fn num(v: &V) -> Result<u32> { v.as_u64().and_then(|n| n.try_into().ok()).ok_or_else(|| Error("QASM metadata: expected u32".into())) }
fn fields(v: &V, len: usize) -> Result<&[V]> { let x=list(v)?; if x.len()!=len { return Err(Error("QASM metadata: wrong field count".into())); } Ok(x) }
fn role(r: Role) -> V { s(if r==Role::Mut {"mut"} else {"in"}) }
fn rrole(v:&V)->Result<Role> { match st(v)?.as_str() { "mut"=>Ok(Role::Mut),"in"=>Ok(Role::In),_=>Err(Error("QASM metadata: bad role".into())) } }
fn q(q:&QubitRef)->V { a([s(&q.bundle), n(q.index)]) }
fn rq(v:&V)->Result<QubitRef> { let x=fields(v,2)?; Ok(QubitRef::new(&st(&x[0])?,num(&x[1])?)) }
fn cs(c:&[Ctrl])->V { a(c.iter().map(|c|a([q(&c.qubit),s(if c.positive {"+"} else {"-"})]))) }
fn rcs(v:&V)->Result<Vec<Ctrl>> { list(v)?.iter().map(|v|{let x=fields(v,2)?; let positive=match st(&x[1])?.as_str(){"+"=>true,"-"=>false,_=>return Err(Error("QASM metadata: bad polarity".into()))};Ok(Ctrl{qubit:rq(&x[0])?,positive})}).collect() }
pub(crate) fn gate(g:BaseGate)->String { match g { BaseGate::X=>"x".into(),BaseGate::H=>"h".into(),BaseGate::S=>"s".into(),BaseGate::Sdg=>"sdg".into(),BaseGate::T=>"t".into(),BaseGate::Tdg=>"tdg".into(),BaseGate::Cz=>"cz".into(),BaseGate::Swap=>"swap".into(),BaseGate::Rx(t)=>format!("rx({t})"),BaseGate::Ry(t)=>format!("ry({t})"),BaseGate::Rz(t)=>format!("rz({t})") } }
pub(crate) fn read_gate(s:&str)->Result<BaseGate> {
    Ok(match s {"x"=>BaseGate::X,"h"=>BaseGate::H,"s"=>BaseGate::S,"sdg"=>BaseGate::Sdg,"t"=>BaseGate::T,"tdg"=>BaseGate::Tdg,"cz"=>BaseGate::Cz,"swap"=>BaseGate::Swap,_=>{
        let (op,t)=s.split_once('(').ok_or_else(||Error(format!("QASM: unknown gate {s}")))?;
        let t:f64=t.strip_suffix(')').ok_or_else(||Error("QASM: missing )".into()))?.parse().map_err(|_|Error("QASM: invalid angle".into()))?;
        if !t.is_finite(){return Err(Error("QASM: nonfinite angle".into()));}
        match op {"rx"=>BaseGate::Rx(t),"ry"=>BaseGate::Ry(t),"rz"=>BaseGate::Rz(t),_=>return Err(Error(format!("QASM: unknown gate {s}")))}
    }})
}
fn node(v:&Node)->V { match v {
 Node::Gate(g)=>a([s("gate"),s(gate(g.gate)),cs(&g.controls),a(g.targets.iter().map(q))]),
 Node::Call{callee,adjoint,controls,args}=>a([s("call"),s(callee),V::Bool(*adjoint),cs(controls),a(args.iter().map(s))]),
 Node::Copy{src,dst,controls}=>a([s("copy"),s(src),s(dst),cs(controls)]),
 Node::Uncopy{src,dst,controls}=>a([s("uncopy"),s(src),s(dst),cs(controls)]),
 Node::Xor{src,tgt,controls}=>a([s("xor"),s(src),s(tgt),cs(controls)]),
 Node::Exch{a:x,b,controls}=>a([s("exch"),s(x),s(b),cs(controls)]),
 Node::Load{region,addr,dst,controls}=>a([s("load"),s(region),s(addr),s(dst),cs(controls)]),
 Node::Unload{region,addr,dst,controls}=>a([s("unload"),s(region),s(addr),s(dst),cs(controls)]),
 Node::Store{region,addr,val,controls}=>a([s("store"),s(region),s(addr),s(val),cs(controls)]),
 Node::ExchM{region,addr,val,controls}=>a([s("exchm"),s(region),s(addr),s(val),cs(controls)]),
} }
fn rnode(v:&V)->Result<Node> { let x=list(v)?;let kind=st(x.first().ok_or_else(||Error("QASM metadata: empty node".into()))?)?;
 if kind=="call" { fields(v,5)?;return Ok(Node::Call{callee:st(&x[1])?,adjoint:match x[2]{V::Bool(b)=>b,_=>return Err(Error("QASM metadata: expected bool".into()))},controls:rcs(&x[3])?,args:list(&x[4])?.iter().map(st).collect::<Result<_>>()?}); }
 if kind=="gate" { fields(v,4)?;return Ok(Node::Gate(GateInst{gate:read_gate(&st(&x[1])?)?,controls:rcs(&x[2])?,targets:list(&x[3])?.iter().map(rq).collect::<Result<_>>()?})); }
 let mem=matches!(kind.as_str(),"load"|"unload"|"store"|"exchm");fields(v,if mem {5}else{4})?;
 let a=st(&x[1])?;let b=st(&x[2])?;let controls=rcs(&x[if mem {4}else{3}])?;
 Ok(match kind.as_str(){
 "copy"=>Node::Copy{src:a,dst:b,controls},"uncopy"=>Node::Uncopy{src:a,dst:b,controls},"xor"=>Node::Xor{src:a,tgt:b,controls},"exch"=>Node::Exch{a,b,controls},
 "load"=>Node::Load{region:a,addr:b,dst:st(&x[3])?,controls},"unload"=>Node::Unload{region:a,addr:b,dst:st(&x[3])?,controls},"store"=>Node::Store{region:a,addr:b,val:st(&x[3])?,controls},"exchm"=>Node::ExchM{region:a,addr:b,val:st(&x[3])?,controls},_=>return Err(Error("QASM metadata: unknown node".into()))})
}
fn bundles(bs:&[Bundle])->V { a(bs.iter().map(|b|a([s(&b.name),n(b.width),role(b.role)]))) }
fn rbundles(v:&V)->Result<Vec<Bundle>> { list(v)?.iter().map(|v|{let x=fields(v,3)?;Ok(Bundle{name:st(&x[0])?,width:num(&x[1])?,role:rrole(&x[2])?})}).collect() }
pub(crate) fn encode(p:&Program)->V { a([s("bc-ir-1"),s(&p.top),a(p.circuits.iter().map(|c|a([
 s(&c.name),V::Bool(c.perm),bundles(&c.mut_bundles),bundles(&c.in_bundles),
 a(c.mem.iter().map(|m|a([s(&m.name),n(m.addr_width),n(m.cell_width),role(m.role)]))),
 a(c.workspace.iter().map(|w|a([s(&w.name),n(w.width),s(match w.kind{WsKind::Clean=>"clean",WsKind::Borrowed=>"borrowed",WsKind::LocalRegion=>"region"}),s(&w.class)]))),
 a(c.local_regions.iter().map(|r|a([s(&r.name),n(r.addr_width),n(r.cell_width)]))),a(c.body.iter().map(node))
 ])))]) }
pub(crate) fn decode(v:&V)->Result<Program> { let p=fields(v,3)?;if st(&p[0])?!="bc-ir-1"{return Err(Error("QASM metadata: bad version".into()));}
 let circuits=list(&p[2])?.iter().map(|v|{let c=fields(v,8)?;Ok(Circuit{
 name:st(&c[0])?,perm:match c[1]{V::Bool(b)=>b,_=>return Err(Error("QASM metadata: expected bool".into()))},mut_bundles:rbundles(&c[2])?,in_bundles:rbundles(&c[3])?,
 mem:list(&c[4])?.iter().map(|v|{let m=fields(v,4)?;Ok(MemBundle{name:st(&m[0])?,addr_width:num(&m[1])?,cell_width:num(&m[2])?,role:rrole(&m[3])?})}).collect::<Result<_>>()?,
 workspace:list(&c[5])?.iter().map(|v|{let w=fields(v,4)?;Ok(Workspace{name:st(&w[0])?,width:num(&w[1])?,kind:match st(&w[2])?.as_str(){"clean"=>WsKind::Clean,"borrowed"=>WsKind::Borrowed,"region"=>WsKind::LocalRegion,_=>return Err(Error("QASM metadata: bad workspace kind".into()))},class:st(&w[3])?})}).collect::<Result<_>>()?,
 local_regions:list(&c[6])?.iter().map(|v|{let r=fields(v,3)?;Ok(RegionInfo{name:st(&r[0])?,addr_width:num(&r[1])?,cell_width:num(&r[2])?})}).collect::<Result<_>>()?,body:list(&c[7])?.iter().map(rnode).collect::<Result<_>>()?,
 })}).collect::<Result<_>>()?;
 Ok(Program{top:st(&p[1])?,circuits})
}
