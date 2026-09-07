//! 行级文本 diff（经典 LCS 动态规划实现，无外部依赖）。
//!
//! 用于「模板更新」弹窗的类代码块展示：把本地实例与远端模板各序列化成规范 JSON，
//! 再逐行比对产出 `-`/`+`/上下文 三种行，前端按行着色。

/// 行类型：`+` 新增、`-` 删除、空格 上下文
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DiffLine {
    pub kind: char,
    pub text: String,
}

impl DiffLine {
    pub fn is_changed(&self) -> bool {
        self.kind == '+' || self.kind == '-'
    }
}

/// 对 old/new 两个多行文本做 LCS 行级 diff，返回带类型前缀的行序列。
/// O(n*m) 空间与时间；模板 JSON 通常一两百行，开销可忽略。
pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    let n = a.len();
    let m = b.len();

    // dp[i][j] = a[i..] 与 b[j..] 的 LCS 长度
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity((n + m).max(1).saturating_div(2) + 1);
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push(DiffLine { kind: ' ', text: a[i].to_string() });
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(DiffLine { kind: '-', text: a[i].to_string() });
            i += 1;
        } else {
            out.push(DiffLine { kind: '+', text: b[j].to_string() });
            j += 1;
        }
    }
    while i < n {
        out.push(DiffLine { kind: '-', text: a[i].to_string() });
        i += 1;
    }
    while j < m {
        out.push(DiffLine { kind: '+', text: b[j].to_string() });
        j += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_is_all_context() {
        let lines = diff_lines("a\nb\nc", "a\nb\nc");
        assert!(lines.iter().all(|l| l.kind == ' '));
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn inserted_and_removed_lines_are_marked() {
        let lines = diff_lines("x\nkeep\n", "x\nnew\nkeep\n");
        assert!(lines.iter().any(|l| l.kind == '+' && l.text == "new" && l.is_changed()));
        assert!(!lines.iter().any(|l| l.kind == '-' && l.is_changed()));
    }

    #[test]
    fn replaced_lines_flip_to_minus_plus() {
        let lines = diff_lines("a\nold\nz\n", "a\nnew\nz\n");
        assert!(lines.iter().any(|l| l.kind == '-' && l.text == "old"));
        assert!(lines.iter().any(|l| l.kind == '+' && l.text == "new"));
    }
}