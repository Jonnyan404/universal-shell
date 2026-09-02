fn main() {
    use shared::ShellConfig;
    let cfg = ShellConfig::load(&std::path::PathBuf::from("/Users/jonny/Documents/GitHub/universal-shell/demo/shell.json")).unwrap();
    assert_eq!(cfg.programs.len(), 2);
    let cc = &cfg.programs[0];
    assert_eq!(cc.id, "cloud-clipboard-go");
    assert!(cc.binary == "cloud-clipboard-go");
    let (rule, candidates) = cc.candidate_names("aarch64", "5.0.1").unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0], "cloud-clipboard-go_Darwin_arm64.tar.gz");
    assert_eq!(rule.member.as_deref(), Some("cloud-clipboard-go"));
    assert_eq!(rule.mode, shared::config::ExtractMode::Single);
    let vals = cc.runtime_defaults();
    assert_eq!(vals.get("port").unwrap(), "9000");
    assert_eq!(vals.get("verbose").unwrap(), "false");
    let rv = definitions(&cc);
    let args = cc.render_args(&rv);
    assert_eq!(args, vec!["-host", "0.0.0.0", "-port", "9000", "-config", "", "-dir", ""]);
    println!("smoke ok: fields={} default_port={} mode={:?} args={:?}", cc.fields.len(), vals.get("port").unwrap(), rule.mode, args);
}

fn definitions(p: &shared::Program) -> std::collections::BTreeMap<String, String> {
    p.fields.iter().map(|f| (f.key.clone(), f.default_raw())).collect()
}