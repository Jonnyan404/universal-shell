//! 内置受管程序：编译期嵌入，应用一打开即可用，无需 shell.json、无需联网加载模板库。
//!
//! 程序清单在 `builtin/programs.json`，用 `include_str!` 编译进二进制。想加更多内置程序，
//! 只需在 `programs.json` 的 `programs` 数组里追加即可（预留扩展）。

use std::sync::OnceLock;

use crate::config::Program;

const BUILTIN_JSON: &str = include_str!("../builtin/programs.json");

/// 内嵌文件的外层 wrapper（承载 programs 数组，便于后续扩展其它字段）。
#[derive(serde::Deserialize)]
struct BuiltinFile {
    #[serde(default)]
    programs: Vec<Program>,
}

/// 编译期内嵌程序清单的解析结果（懒加载一次）。
struct BuiltinSet {
    programs: Vec<Program>,
}

fn builtin_set() -> &'static BuiltinSet {
    static SET: OnceLock<BuiltinSet> = OnceLock::new();
    SET.get_or_init(|| {
        let file: BuiltinFile = serde_json::from_str(BUILTIN_JSON)
            .expect("builtin/programs.json 格式错误，请检查模板");
        BuiltinSet {
            programs: file.programs,
        }
    })
}

/// 全部内置受管程序（编译期固定）。
pub fn builtin_programs() -> &'static [Program] {
    &builtin_set().programs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_programs_parse_and_are_valid() {
        let ps = builtin_programs();
        assert!(!ps.is_empty(), "应有至少一个内置程序");
        let mut ids = std::collections::HashSet::new();
        for p in ps {
            assert!(ids.insert(p.id.clone()), "内置程序 id 重复: {}", p.id);
            assert!(!p.id.is_empty() && !p.name.is_empty() && !p.repo.is_empty() && !p.binary.is_empty());
        }
    }
}

