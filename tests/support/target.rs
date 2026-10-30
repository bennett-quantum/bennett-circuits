use bennett_circuits::{json::Value as V, target::Target};
pub fn universal(qram: bool, schemes: bool) -> Target {
    let mut v=V::parse_target(r#"{"capabilities":{"classical_step_time":1,"max_parallel":"unbounded","max_predicate_depth":"unbounded","mbu":false,"measurement":{"feedforward_latency":0,"mid_circuit":false}},"classes":{"fast":{"capacity":"unbounded","dirty_rate":1,"qubit_second":1}},"gates":[],"imports":{},"objective":{"weighted":{"time":1}},"ptr_widths_supported":[8,16,32,64],"qram":null,"resources":{"gates":{"kind":"count","unit":"gate"},"peak_qubits":{"kind":"peak","unit":"qubit"},"qubit_second":{"kind":"occupancy","unit":"qs"},"time":{"kind":"time","unit":"s"}},"schemes":[],"tables":{}}"#).unwrap();
    let V::Object(m) = &mut v else { unreachable!() };
    m.insert("gates".into(),V::Array(["x","cx","ccx","mcx","h","s","sdg","t","tdg","cz","swap","rx","ry","rz"].iter().map(|op|{
        let count=if ["cz","swap"].contains(op){2}else{1};
        let pattern=V::Array((0..count).map(|_|V::Str("*".into())).collect());
        let mut g=V::parse(r#"{"controls":{"min":0,"max":"unbounded"},"epsilon":0,"cost":{"time":1,"gates":1}}"#).unwrap();let V::Object(gm)=&mut g else{unreachable!()};gm.insert("op".into(),V::Str((*op).into()));gm.insert("classes".into(),pattern);
        if schemes && *op=="x" {gm.insert("controls".into(),V::parse(r#"{"min":0,"max":0}"#).unwrap());}
        if ["rx","ry","rz"].contains(op){gm.insert("params".into(),V::Array(vec![V::Str("theta".into())]));}g
    }).collect()));
    if qram {
        m.insert("qram".into(),V::parse(r#"{"load":{"cost":{"time":2,"gates":1}},"store":{"cost":{"time":3,"gates":1}},"exch":{"cost":{"time":4,"gates":1}}}"#).unwrap());
    }
    if schemes {
        m.insert("schemes".into(),V::Array((1..16).map(|c|{
        let mut s=V::parse(r#"{"name":"cx_wrapper","realises":{"op":"x","controls":{"min":1,"max":1}},"ancillae":{"count":0,"class":"fast","state":"clean"},"template":[]}"#).unwrap();let V::Object(sm)=&mut s else{unreachable!()};sm.insert("name".into(),V::Str(format!("controlled_x_{c}")));sm.insert("realises".into(),V::parse(&format!(r#"{{"op":"x","controls":{{"min":{c},"max":{c}}}}}"#)).unwrap());
        let mut wires=(0..c).map(|i|V::Str(format!("c[{i}]"))).collect::<Vec<_>>();wires.push(V::Str("t".into()));
        sm.insert("template".into(),V::Array(vec![V::Object(std::collections::BTreeMap::from([("gate".into(),V::Str("mcx".into())),("wires".into(),V::Array(wires))]))]));s
    }).collect()));
    }
    Target::parse(&v.to_jcs()).unwrap()
}
