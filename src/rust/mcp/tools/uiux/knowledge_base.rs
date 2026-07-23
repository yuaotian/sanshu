// UI/UX 知识库物化模块
// 意图：把编译期内嵌的 ui-ux-pro-max-skill.md 幂等落盘到稳定目录
// （<系统配置目录>/sanshu/uiux-knowledge/），使 fast-context 这类基于
// 文件系统的检索后端可以对知识库做定向精确检索。
// 背景：旧实现尝试在"用户目标项目"里用 sou 搜索该文件，但文件只内嵌于
// 三术自身，目标项目必然不存在，导致知识检索永远降级到本地朴素匹配。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

/// 与 markdown_search 共用同一份内嵌知识库内容，保证两条检索路径数据一致
const UIUX_MARKDOWN: &str = include_str!("../../../assets/resources/ui-ux-pro-max-skill.md");

/// 知识库文件名（fast-context 命中结果按该文件名过滤）
pub const UIUX_MARKDOWN_FILENAME: &str = "ui-ux-pro-max-skill.md";

/// 知识库目录：<系统配置目录>/sanshu/uiux-knowledge
fn knowledge_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().ok_or_else(|| anyhow!("无法定位系统配置目录"))?;
    Ok(base.join("sanshu").join("uiux-knowledge"))
}

/// 幂等物化知识库：内容一致时跳过写入，首次或内容变更（升级）时覆盖。
/// 返回知识库目录的正斜杠路径，可直接作为 sou/fast-context 的 project_root_path。
pub fn ensure_materialized() -> Result<String> {
    let dir = knowledge_dir()?;
    materialize_into(&dir)
}

/// 实际物化逻辑，目录参数独立出来便于单元测试
fn materialize_into(dir: &Path) -> Result<String> {
    fs::create_dir_all(dir).with_context(|| format!("创建知识库目录失败: {}", dir.display()))?;

    let file_path = dir.join(UIUX_MARKDOWN_FILENAME);
    // 中文说明：先比对现有内容，一致则跳过写入，避免每次调用都触碰磁盘 mtime
    let up_to_date = fs::read_to_string(&file_path)
        .map(|existing| existing == UIUX_MARKDOWN)
        .unwrap_or(false);
    if !up_to_date {
        fs::write(&file_path, UIUX_MARKDOWN)
            .with_context(|| format!("写入知识库文件失败: {}", file_path.display()))?;
    }

    Ok(dir.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn materialize_writes_file_and_is_idempotent() {
        let temp = tempdir().expect("临时目录应创建成功");
        let dir = temp.path().join("uiux-knowledge");

        // 首次物化：文件应存在且内容与内嵌一致
        let returned = materialize_into(&dir).expect("首次物化应成功");
        let file_path = dir.join(UIUX_MARKDOWN_FILENAME);
        assert!(file_path.exists(), "物化后知识库文件应存在");
        assert_eq!(
            fs::read_to_string(&file_path).expect("知识库文件应可读"),
            UIUX_MARKDOWN
        );
        assert!(!returned.contains('\\'), "返回路径应为正斜杠形式");

        // 第二次物化：应幂等成功且内容不变
        materialize_into(&dir).expect("重复物化应幂等成功");
        assert_eq!(
            fs::read_to_string(&file_path).expect("知识库文件应可读"),
            UIUX_MARKDOWN
        );
    }

    #[test]
    fn materialize_repairs_stale_content() {
        let temp = tempdir().expect("临时目录应创建成功");
        let dir = temp.path().join("uiux-knowledge");
        fs::create_dir_all(&dir).expect("目录应创建成功");
        let file_path = dir.join(UIUX_MARKDOWN_FILENAME);
        // 模拟旧版本残留内容，物化后应被最新内嵌内容覆盖
        fs::write(&file_path, "旧版本内容").expect("写入旧内容应成功");

        materialize_into(&dir).expect("物化应成功");
        assert_eq!(
            fs::read_to_string(&file_path).expect("知识库文件应可读"),
            UIUX_MARKDOWN
        );
    }
}
