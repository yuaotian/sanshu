//! 文本相似度计算模块
//!
//! 提供多种文本相似度算法，用于记忆去重检测
//! 参考 Java 项目 similarity-master 中的算法实现

use std::collections::HashSet;

/// 文本相似度计算器
pub struct TextSimilarity;

impl TextSimilarity {
    /// 综合相似度（组合多种算法）
    ///
    /// 权重分配：
    /// - 编辑距离相似度: 0.4（捕捉字符级别的相似性）
    /// - 短语相似度: 0.4（考虑字符位置关系）
    /// - Jaccard 字符集: 0.2（捕捉字符集合重叠）
    pub fn calculate(s1: &str, s2: &str) -> f64 {
        let norm1 = Self::normalize(s1);
        let norm2 = Self::normalize(s2);

        // 精确匹配快速返回
        if norm1 == norm2 {
            return 1.0;
        }

        let lev = Self::levenshtein_similarity(&norm1, &norm2);
        let phrase = Self::phrase_similarity(&norm1, &norm2);
        let jaccard = Self::jaccard_char_similarity(&norm1, &norm2);

        // 加权综合
        lev * 0.4 + phrase * 0.4 + jaccard * 0.2
    }

    /// 增强版综合相似度
    ///
    /// 结合多种算法 + 子串包含检测
    /// 取各算法的最高分作为最终结果
    pub fn calculate_enhanced(s1: &str, s2: &str) -> f64 {
        let basic = Self::calculate(s1, s2);
        let contains = Self::contains_similarity(s1, s2);

        // 取两者中的较大值
        basic.max(contains)
    }

    /// 编辑距离相似度 (Levenshtein)
    ///
    /// 计算将一个字符串转换为另一个字符串所需的最小编辑操作数
    /// 相似度 = 1.0 - (编辑距离 / 最大长度)
    pub fn levenshtein_similarity(s1: &str, s2: &str) -> f64 {
        let dist = Self::levenshtein_distance(s1, s2);
        let max_len = s1.chars().count().max(s2.chars().count());
        if max_len == 0 {
            return 1.0;
        }
        1.0 - (dist as f64 / max_len as f64)
    }

    /// 编辑距离计算（动态规划实现，使用滚动数组优化空间）
    fn levenshtein_distance(s1: &str, s2: &str) -> usize {
        let a: Vec<char> = s1.chars().collect();
        let b: Vec<char> = s2.chars().collect();
        let n = a.len();
        let m = b.len();

        if n == 0 {
            return m;
        }
        if m == 0 {
            return n;
        }

        // 使用滚动数组优化空间复杂度 O(min(n,m))
        let mut prev = vec![0usize; m + 1];
        let mut curr = vec![0usize; m + 1];

        // 初始化第一行
        for j in 0..=m {
            prev[j] = j;
        }

        for i in 1..=n {
            curr[0] = i;
            for j in 1..=m {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                curr[j] = (prev[j] + 1) // 删除
                    .min(curr[j - 1] + 1) // 插入
                    .min(prev[j - 1] + cost); // 替换
            }
            std::mem::swap(&mut prev, &mut curr);
        }

        prev[m]
    }

    /// 短语相似度
    ///
    /// 参考 Java similarity-master 项目中的 PhraseSimilarity.java
    /// 基于相同字符和位置距离加权计算
    pub fn phrase_similarity(s1: &str, s2: &str) -> f64 {
        let chars1: Vec<char> = s1.chars().collect();
        let chars2: Vec<char> = s2.chars().collect();

        if chars1.is_empty() && chars2.is_empty() {
            return 1.0;
        }
        if chars1.is_empty() || chars2.is_empty() {
            return 0.0;
        }

        // 双向计算取平均
        (Self::get_sc(&chars1, &chars2) + Self::get_sc(&chars2, &chars1)) / 2.0
    }

    /// 计算 first 相对于 second 的相似度贡献
    fn get_sc(first: &[char], second: &[char]) -> f64 {
        let mut total = 0.0;
        for pos in 0..first.len() {
            total += Self::get_cc(first, second, pos);
        }
        total / first.len() as f64
    }

    /// 计算单个字符的相似度贡献
    fn get_cc(first: &[char], second: &[char], pos: usize) -> f64 {
        let d = Self::get_distance(first, second, pos);
        (second.len() - d) as f64 / second.len() as f64
    }

    /// 计算字符的最小位置距离
    fn get_distance(first: &[char], second: &[char], pos: usize) -> usize {
        let ch = first[pos];
        let mut min_dist = second.len();
        for (i, &c) in second.iter().enumerate() {
            if c == ch {
                let dist = if i > pos { i - pos } else { pos - i };
                min_dist = min_dist.min(dist);
            }
        }
        min_dist
    }

    /// Jaccard 字符集相似度
    ///
    /// 计算两个字符串字符集合的 Jaccard 系数
    /// 公式: |A ∩ B| / |A ∪ B|
    pub fn jaccard_char_similarity(s1: &str, s2: &str) -> f64 {
        let set1: HashSet<char> = s1.chars().collect();
        let set2: HashSet<char> = s2.chars().collect();

        if set1.is_empty() && set2.is_empty() {
            return 1.0;
        }

        let intersection = set1.intersection(&set2).count();
        let union = set1.union(&set2).count();

        if union == 0 {
            return 0.0;
        }

        intersection as f64 / union as f64
    }

    /// 子串包含检测
    ///
    /// 检测短文本是否被长文本完全包含
    /// 如果短文本是长文本的子串，返回较高的相似度
    ///
    /// 返回值：
    /// - 0.8 ~ 1.0: 完全包含（根据长度比例）
    /// - 0.0: 不包含
    pub fn contains_similarity(s1: &str, s2: &str) -> f64 {
        let norm1 = Self::normalize(s1);
        let norm2 = Self::normalize(s2);

        if norm1.is_empty() || norm2.is_empty() {
            return 0.0;
        }

        // 判断谁是短文本
        let (short, long) = if norm1.len() <= norm2.len() {
            (&norm1, &norm2)
        } else {
            (&norm2, &norm1)
        };

        // 如果短文本完全包含在长文本中
        if long.contains(short.as_str()) {
            // 根据长度比例给予相似度
            // 短文本越接近长文本长度，相似度越高
            let ratio = short.chars().count() as f64 / long.chars().count() as f64;
            // 基础包含得分 0.8，加上长度比例加成
            return (0.8 + 0.2 * ratio).min(1.0);
        }

        0.0
    }

    /// 文本归一化
    ///
    /// 预处理步骤：
    /// 1. 转换为小写（仅英文）
    /// 2. 合并连续空白字符为单个空格
    /// 3. 去除首尾空白
    /// 4. 移除常见标点符号
    pub fn normalize(text: &str) -> String {
        let mut result = String::new();
        let mut prev_is_space = true; // 用于合并连续空白

        for ch in text.chars() {
            if ch.is_whitespace() {
                if !prev_is_space {
                    result.push(' ');
                    prev_is_space = true;
                }
            } else if Self::is_punctuation(ch) {
                // 跳过标点符号
                continue;
            } else {
                // 中文不转小写，英文转小写
                if ch.is_ascii_alphabetic() {
                    result.push(ch.to_ascii_lowercase());
                } else {
                    result.push(ch);
                }
                prev_is_space = false;
            }
        }

        result.trim().to_string()
    }

    /// 判断是否为标点符号
    fn is_punctuation(ch: char) -> bool {
        // 注意：ASCII 双引号 '"' 已在第一行包含，这里只添加中文标点
        matches!(
            ch,
            '.' | ',' | '!' | '?' | ';' | ':' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}'
                | '。'
                | '，'
                | '！'
                | '？'
                | '；'
                | '：'
                | '\u{201C}' // 中文左双引号 "
                | '\u{201D}' // 中文右双引号 "
                | '\u{2018}' // 左单引号 '
                | '\u{2019}' // 右单引号 '
                | '（'
                | '）'
                | '【'
                | '】'
                | '、'
                | '·'
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!((TextSimilarity::calculate_enhanced("使用 KISS 原则", "使用 KISS 原则") - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_space_difference() {
        let sim = TextSimilarity::calculate_enhanced("使用 KISS 原则", "使用KISS原则");
        assert!(sim > 0.80, "相似度应该 > 80%: {}", sim);
    }

    #[test]
    fn test_similar_expression() {
        let sim = TextSimilarity::calculate_enhanced("使用 KISS 原则", "遵循 KISS 原则");
        assert!(sim > 0.70, "相似度应该 > 70%: {}", sim);
    }

    #[test]
    fn test_substring_detection() {
        let sim = TextSimilarity::calculate_enhanced("KISS", "使用 KISS 原则");
        assert!(sim > 0.80, "子串检测应该 > 80%: {}", sim);
    }

    #[test]
    fn test_unrelated() {
        let sim = TextSimilarity::calculate_enhanced("使用 KISS 原则", "配置数据库连接");
        assert!(sim < 0.50, "不相关文本相似度应该 < 50%: {}", sim);
    }
}
