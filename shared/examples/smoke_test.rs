fn main() {
    // 内置程序编译期嵌入，直接测 cloud-clipboard-go 的字段/资产/参数渲染
    let cc = shared::builtin::builtin_programs()
        .iter()
        .find(|p| p.id == "cloud-clipboard-go")
        .expect("内置应有 cloud-clipboard-go");
    assert_eq!(cc.binary, "cloud-clipboard-go");
    let (rule, candidates) = cc.candidate_names("aarch64", "5.0.1").unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0], "cloud-clipboard-go_Darwin_arm64.tar.gz");
    assert_eq!(rule.member.as_deref(), Some("cloud-clipboard-go"));
    assert_eq!(rule.mode, shared::config::ExtractMode::Single);
    let vals = cc.runtime_defaults();
    assert_eq!(vals.get("port").unwrap(), "9000");
    let rv = definitions(cc);
    let args = cc.render_args(&rv);
    assert_eq!(args, vec!["-host", "127.0.0.1", "-port", "9000", "-config", ""]);
    println!(
        "smoke ok: fields={} default_port={} mode={:?} args={:?}",
        cc.fields.len(),
        vals.get("port").unwrap(),
        rule.mode,
        args
    );
}

fn definitions(p: &shared::Program) -> std::collections::BTreeMap<String, String> {
    p.fields.iter().map(|f| (f.key.clone(), f.default_raw())).collect()
}
