//! UI/UX 轻量词典与中英文映射
//!
//! 设计目标：
//! - 为 BM25 提供查询扩展，在不引入 embedding 的前提下改善中英文混合召回
//!
//! 约束：
//! - 不引入重量级 NLP 依赖；保持可维护、可扩展
//! - 词表应“少而精”，优先覆盖高频场景，可随使用反馈迭代

/// Query Expansion：中文短语 → 英文关键词（尽量选择在内嵌 CSV 中高概率出现/有用的 token）。
///
/// 用途：
/// - 纯中文/中英混合输入时，为 BM25 补充英文 token，提高召回
/// - 通过同义概念扩展缓解“现代感 vs 科技感”等语义差异
pub const ZH_TO_EN_EXPANSIONS: &[(&str, &[&str])] = &[
    // 风格/气质（尽量映射到 styles.csv / prompts.csv 常见词）
    // 说明：这里的扩展会影响 BM25 召回，因此优先补充“在 style 语料中也常见”的 token（如 minimalism/swiss）。
    (
        "优雅",
        &[
            "elegant",
            "refined",
            "premium",
            "minimalism",
            "swiss",
            "serif",
        ],
    ),
    ("高级", &["premium", "luxury", "elegant", "minimalism"]),
    ("专业", &["professional", "enterprise", "saas"]),
    ("简约", &["minimal", "minimalism", "clean"]),
    ("极简", &["minimal", "minimalism", "clean"]),
    ("清爽", &["clean", "minimal", "spacious"]),
    ("现代", &["modern", "clean", "minimal"]),
    (
        "科技感",
        &["futuristic", "hud", "technical", "wireframe", "sci-fi"],
    ),
    ("科幻", &["futuristic", "hud", "sci-fi"]),
    ("赛博", &["cyberpunk", "neon", "retro-futurism"]),
    ("霓虹", &["neon", "glow"]),
    ("渐变", &["gradient", "aurora", "mesh"]),
    ("质感", &["premium", "luxury", "glassmorphism"]),
    ("未来", &["futuristic", "neon", "hud"]),
    ("暗黑", &["dark", "oled", "night"]),
    ("深色", &["dark", "oled", "night"]),
    ("黑洞", &["space", "cosmic", "dark", "gravity", "immersive"]),
    ("星球", &["planet", "space", "orbital", "3d"]),
    ("地球", &["earth", "planet", "space", "3d"]),
    ("月球", &["moon", "orbit", "satellite", "space"]),
    ("卫星", &["satellite", "orbit", "telemetry", "space"]),
    ("战舰", &["space", "futuristic", "sci-fi", "3d"]),
    ("公转", &["orbit", "animation", "motion"]),
    ("星空", &["space", "cosmic", "dark", "immersive"]),
    ("裂缝", &["cinematic", "3d", "immersive"]),
    ("吞噬", &["animation", "motion", "cinematic"]),
    ("玻璃", &["glassmorphism", "glass", "blur"]),
    ("毛玻璃", &["glassmorphism", "blur", "glass"]),
    ("新拟物", &["neumorphism", "soft", "embossed"]),
    ("扁平", &["flat", "design"]),
    ("粗野", &["brutalism", "stark"]),
    ("新粗野", &["neubrutalism", "brutalism"]),
    ("复古", &["retro", "vintage"]),
    ("可爱", &["playful", "soft"]),
    ("活泼", &["playful", "vibrant"]),
    ("立体", &["hyperrealism"]),
    // 页面/场景
    ("登录", &["login", "auth", "signin"]),
    ("注册", &["signup", "register"]),
    ("落地页", &["landing", "cta", "hero", "conversion"]),
    ("仪表盘", &["dashboard", "analytics", "kpi"]),
    ("后台管理", &["admin", "dashboard", "enterprise"]),
    ("管理面板", &["admin", "dashboard", "panel"]),
    ("设计系统", &["design", "system", "component", "token"]),
    // 设计要素
    ("配色", &["color", "palette", "contrast"]),
    ("字体", &["typography", "font", "heading"]),
    ("排版", &["typography", "hierarchy", "heading"]),
    ("动效", &["animation", "motion", "transition"]),
    (
        "高性能",
        &["performance", "efficient", "compositor", "canvas"],
    ),
    ("性能", &["performance", "efficient"]),
    ("几何", &["geometric", "wireframe", "grid"]),
    ("交互", &["usability", "navigation", "focus"]),
    ("无障碍", &["accessibility", "wcag", "aria"]),
    ("图表", &["chart", "graph", "visualization"]),
    ("图标", &["icon", "icons", "svg"]),
    ("按钮", &["button", "cta"]),
    ("表单", &["form", "input"]),
    ("间距", &["spacing", "grid"]),
    ("对齐", &["alignment", "grid"]),
];

/// Query Expansion：英文 token → 同义/相关 token（轻量补充，避免过度扩展）。
pub const EN_SYNONYMS: &[(&str, &[&str])] = &[
    ("modern", &["contemporary", "clean", "minimal"]),
    ("futuristic", &["neon", "cyberpunk", "hud"]),
    ("elegant", &["refined", "premium", "luxury"]),
    ("glassmorphism", &["glass", "blur", "liquid"]),
    ("neumorphism", &["soft", "embossed", "debossed"]),
    ("aurora", &["gradient", "mesh", "iridescent"]),
    ("cyberpunk", &["neon", "hud", "vaporwave", "retro-futurism"]),
    ("brutalism", &["neubrutalism", "stark", "raw"]),
    ("dashboard", &["analytics", "kpi", "chart"]),
    ("landing", &["hero", "cta", "conversion"]),
    ("login", &["auth", "signin"]),
    (
        "direct2d",
        &["canvas", "native", "rendering", "performance"],
    ),
    ("canvas", &["rendering", "animation", "performance"]),
    ("space", &["cosmic", "aerospace", "orbital"]),
];
