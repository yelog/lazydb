# Completion Detail Type Alignment Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 SQL Editor 与数据网格补全弹框中的列类型显示为方言通用的短拼写（`character varying(30)` → `varchar(30)`、`timestamp without time zone` → `timestamp`），并作为独立列右对齐；label 左端仍与编辑器中正在输入的标识符对齐，弹框宽度不再被冗长类型名撑开。

**Architecture:** 在 `src/sql` 新增纯函数 `short_type_name`，只在两个展示字段接缝（`CompletionCandidate.detail`、`DataQueryCandidate.type_name`）调用；catalog 的 `ColumnMetadata.native_type` 与 `ResultSet.columns[].type_name` 保持原样，因为单元格编辑值解析（`src/app.rs` 的 `parse_relation_value`）依赖其语义。渲染层把"label 后紧跟 detail"改成 icon / label / detail 三列表格：列宽先按候选内容度量，再按 clamp 后的实际弹框内宽收敛（label 优先，detail 吃剩余空间），两个补全弹框复用同一套列计算与行拼装。

**Tech Stack:** Rust 2024、ratatui、unicode-width、现有 `CompletionIndex`/`CatalogEntry` 模型、Cargo unit + integration tests、rustfmt、Clippy

---

## 成功标准

- Postgres 列候选的类型显示为 `varchar(30)`、`varchar(200)`、`timestamp`、`bigint`，不再出现 `character varying`、`without time zone`。
- 同一弹框内所有类型串的**右端列号相同**，且距弹框右边框恒为 1 格留白。
- label 左端仍与编辑器中正在输入的标识符对齐（`popup_x` 锚点行为不变）。
- 选中行高亮为覆盖弹框内宽的完整矩形，右端不再随该行文字长度参差。
- 视口空间不足时：label 优先完整显示，type 列先截断（带 `…`）、再整列隐藏，绝不出现被边框切掉的半截类型。
- MySQL `enum('a','b')` 显示为 `enum`；`numeric(10,2)`、`bigint`、`jsonb`、`uuid` 等已简短的类型保持原样。
- 未识别的类型名（含被注入控制字符的名字）原样透传，`tests/sql_completion.rs` 现有 `text<ESC>[31m` 断言继续通过。
- 数据网格 WHERE / ORDER BY 补全弹框获得同样的对齐与简写。
- `cargo test --all-targets --all-features` 与 `cargo clippy --all-targets --all-features -- -D warnings` 全绿。

## 范围边界

### 必须交付

- `src/sql/type_name.rs`：`short_type_name` 纯函数 + 单测。
- 两个展示接缝接入短拼写。
- 补全弹框三列布局 + type 列右对齐 + 整行背景补齐。
- 两个补全弹框复用共享的列宽计算与行拼装。
- `docs/architecture.md` 中"仅用于展示的净化"约束补充短拼写。

### 本轮明确不做

- 不修改 `ColumnMetadata.native_type` / `ResultSet.columns[].type_name` 的原始值。
- 不为"显示原始类型"增加配置开关。
- 不把 `bigint`/`smallint` 改成 `int8`/`int2` 这类 Postgres 内部拼写。
- 不做类型名大小写归一化（SQLite 声明类型的 `TEXT`/`INTEGER` 混排不在本轮处理）。
- 不剥离 Postgres 用户自定义类型的 schema 限定名（`extensions.citext` 保持原样，避免同名歧义）。
- 不改动 Explorer / Structure 视图、DDL 预览、catalog editor 中的类型展示。
- 不改 label 的截断行为（超宽 label 仍由 ratatui 裁切），也不改候选排序、数量上限与触发时机。

## 设计约束

- `label_offset` 必须保持 `icon_width + 1`：`popup_x` 靠它反推候选 label 与编辑器标识符的对齐（`src/ui/mod.rs:996`），改动会破坏 `completion_candidate_label_aligns_with_identifier_start`、`completion_candidate_labels_share_a_fixed_icon_column`、`completion_popup_stays_fixed_while_typing`。
- 列宽必须用 `completion_popup_rect` clamp 之后的 `area.width` 二次收敛。只按 desired 排版时，右对齐的类型会被 ratatui 直接切掉，视觉上表现为"类型消失但留下一片空白"。
- `short_type_name` 保持纯函数、无 UI/运行时依赖，符合 `src/sql` 的 "Pure SQL text services" 定位。
- 短拼写必须在 `display_text`（terminal 净化）之前作用于原始类型名，净化仍是展示路径的最后一步。
- 规则必须保守：只重写识别得到的拼写，任何不匹配的输入原样返回。
- 所有宽度运算使用 `saturating_*`，禁止裸减法。

---

### Task 1: 用单测锁定 `short_type_name` 契约

**Files:**
- Create: `src/sql/type_name.rs`
- Modify: `src/sql/mod.rs:6-19,21-26`
- Test: `src/sql/type_name.rs`（inline `#[cfg(test)] mod tests`）

**Step 1: 创建模块骨架与恒等实现**

新建 `src/sql/type_name.rs`：

```rust
//! Compact display spellings for database native type names.
//!
//! Postgres reports SQL-standard spellings through `format_type`
//! (`character varying(30)`, `timestamp without time zone`), which are too wide
//! for a completion popup. MySQL, SQL Server and SQLite already report compact
//! names, so any unrecognized input is returned unchanged.

/// Returns a compact display spelling for a database native type name.
///
/// The result is display-only. Callers that need the semantic type (value
/// parsing, DDL generation) must keep using the original name.
pub fn short_type_name(value: &str) -> String {
    value.trim().to_owned()
}
```

**Step 2: 注册并导出模块**

在 `src/sql/mod.rs` 的模块列表中按字母序插入 `mod type_name;`（`mod transaction;` 之后），并在 `pub use` 区按字母序加入：

```rust
pub use type_name::short_type_name;
```

**Step 3: 写下映射表单测**

在 `src/sql/type_name.rs` 末尾追加：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postgres_standard_spellings_are_shortened() {
        for (native, expected) in [
            ("character varying(30)", "varchar(30)"),
            ("character varying", "varchar"),
            ("character(4)", "char(4)"),
            ("bpchar", "char"),
            ("bit varying(8)", "varbit(8)"),
            ("double precision", "double"),
            ("boolean", "bool"),
            ("integer", "int"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn time_zone_modifiers_are_folded_into_the_base_name() {
        for (native, expected) in [
            ("timestamp without time zone", "timestamp"),
            ("timestamp with time zone", "timestamptz"),
            ("timestamp(3) without time zone", "timestamp(3)"),
            ("timestamp(3) with time zone", "timestamptz(3)"),
            ("time without time zone", "time"),
            ("time with time zone", "timetz"),
            ("TIMESTAMP WITHOUT TIME ZONE", "timestamp"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn size_arguments_are_kept_and_value_lists_are_dropped() {
        for (native, expected) in [
            ("numeric(10,2)", "numeric(10,2)"),
            ("numeric(10, 2)", "numeric(10, 2)"),
            ("bigint(20) unsigned", "bigint(20) unsigned"),
            ("enum('active','pending')", "enum"),
            ("enum('a(1)','b')", "enum"),
            ("set('a','b')", "set"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn array_suffix_and_unknown_modifiers_survive() {
        for (native, expected) in [
            ("character varying(30)[]", "varchar(30)[]"),
            ("text[]", "text[]"),
            ("interval day to second(6)", "interval day to second(6)"),
            ("int unsigned", "int unsigned"),
        ] {
            assert_eq!(short_type_name(native), expected, "native: {native}");
        }
    }

    #[test]
    fn unrecognized_names_are_returned_verbatim() {
        for native in [
            "bigint",
            "jsonb",
            "uuid",
            "nvarchar",
            "datetime2",
            "extensions.citext",
            // Non-ASCII tail whose byte length would land a time-zone phrase
            // offset inside a character.
            "abc\u{4e2d}xxxxxxxxxxxxx",
            "text\u{1b}[31m",
            "TEXT",
        ] {
            assert_eq!(short_type_name(native), native, "native: {native}");
        }
    }
}
```

`text\u{1b}[31m` 与 `TEXT` 两条是保守性回归：短拼写不得吞掉控制字符，也不得对未命中别名表的名字做大小写改写；`abc中xxxxxxxxxxxxx` 长度恰好落进 `with time zone`（13 字节）的偏移区间，用来钉住时区短语切分的字节边界安全。

**Step 4: 运行单测并确认失败原因**

```bash
cargo test --lib sql::type_name
```

预期失败：`postgres_standard_spellings_are_shortened`、`time_zone_modifiers_are_folded_into_the_base_name`、`size_arguments_are_kept_and_value_lists_are_dropped`、`array_suffix_and_unknown_modifiers_survive` 全部失败（恒等实现原样返回）；`unrecognized_names_are_returned_verbatim` 应当已经通过，作为回归基线。不要为迁就当前实现放宽断言。

### Task 2: 实现类型名解析与短拼写规则

**Files:**
- Modify: `src/sql/type_name.rs`
- Test: `src/sql/type_name.rs`

**Step 1: 加入别名表**

```rust
/// Verbose SQL-standard base names and their compact spellings.
///
/// Matched case-insensitively; unmatched base names keep their original text.
const ALIASES: [(&str, &str); 7] = [
    ("character varying", "varchar"),
    ("character", "char"),
    ("bpchar", "char"),
    ("bit varying", "varbit"),
    ("double precision", "double"),
    ("boolean", "bool"),
    ("integer", "int"),
];
```

**Step 2: 用"数组后缀 → 时区修饰 → 括号参数"的顺序重写函数体**

Postgres 把参数放在修饰语之前（`timestamp(3) without time zone`），因此必须先剥尾缀、再切括号。匹配到时区短语时 base 要转小写，否则 `TIMESTAMP WITHOUT TIME ZONE` 会输出 `TIMESTAMP`（Task 1 的断言是 `timestamp`），`with time zone` 还会拼出 `TIMESTAMPtz`：

```rust
pub fn short_type_name(value: &str) -> String {
    let value = value.trim();
    let (head, array) = match value.strip_suffix("[]") {
        Some(head) => (head.trim_end(), "[]"),
        None => (value, ""),
    };
    let (head, zone) = split_time_zone(head);
    let (base, arguments, modifier) = split_parts(head);
    let base = alias(base);

    let mut short = String::with_capacity(value.len());
    if zone == TimeZone::Absent {
        short.push_str(base);
    } else {
        // A matched time-zone phrase means the spelling was rewritten, so emit
        // the canonical lower-case base instead of mixing cases with `tz`.
        short.push_str(&base.to_ascii_lowercase());
    }
    if zone == TimeZone::Aware {
        short.push_str("tz");
    }
    if let Some(arguments) = arguments.filter(|arguments| is_size_arguments(arguments)) {
        short.push('(');
        short.push_str(arguments);
        short.push(')');
    }
    if !modifier.is_empty() {
        short.push(' ');
        short.push_str(modifier);
    }
    short.push_str(array);
    short
}
```

**Step 3: 加入四个私有辅助函数**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimeZone {
    Absent,
    Naive,
    Aware,
}

/// Splits the trailing `with`/`without time zone` phrase off a type name.
fn split_time_zone(value: &str) -> (&str, TimeZone) {
    for (phrase, zone) in [
        ("without time zone", TimeZone::Naive),
        ("with time zone", TimeZone::Aware),
    ] {
        let Some(split) = value.len().checked_sub(phrase.len()) else {
            continue;
        };
        // `get` rather than `split_at`: the offset is a byte count and type
        // names may end in multi-byte characters.
        if value
            .get(split..)
            .is_some_and(|tail| tail.eq_ignore_ascii_case(phrase))
        {
            return (value[..split].trim_end(), zone);
        }
    }
    (value, TimeZone::Absent)
}

/// Splits a type name into base name, parenthesized arguments and any trailing
/// modifier (`bigint(20) unsigned`).
fn split_parts(value: &str) -> (&str, Option<&str>, &str) {
    let Some(open) = value.find('(') else {
        return (value, None, "");
    };
    let Some(close) = value.rfind(')').filter(|close| *close > open) else {
        return (value, None, "");
    };
    (
        value[..open].trim_end(),
        Some(&value[open + 1..close]),
        value[close + 1..].trim(),
    )
}

fn alias(base: &str) -> &str {
    ALIASES
        .iter()
        .find(|(verbose, _)| base.eq_ignore_ascii_case(verbose))
        .map_or(base, |(_, short)| short)
}

/// Precision/length arguments carry information worth showing; value lists
/// (`enum('a','b')`) do not.
fn is_size_arguments(arguments: &str) -> bool {
    arguments.chars().any(|character| character.is_ascii_digit())
        && arguments
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, ',' | ' '))
}
```

两处细节不要改写：`split_parts` 用 `rfind(')')` 而非 `find(')')`，否则 `enum('a(1)','b')` 会在引号内的括号处切断，把 `','b')` 当成 modifier 拼回去；`split_time_zone` 必须用 `checked_sub` + `value.get(split..)`，不能用 `split_at`——偏移量是字节数，`abc中xxxxxxxxxxxxx` 这类以多字节字符结尾的名字会让 `split_at` 落在字符内部并 panic。

**Step 4: 跑通单测**

```bash
cargo test --lib sql::type_name
```

Task 1 的五个单测必须全绿。若 `interval day to second(6)` 失败，检查 `split_parts` 是否把 `day to second` 误当作 modifier——它属于 base，`find('(')` 之前的整段都是 base。

### Task 3: 把短拼写接入两个展示接缝

**Files:**
- Modify: `src/sql/completion.rs:367-372`
- Modify: `src/app.rs:10153-10159`
- Test: `tests/sql_completion.rs`

**Step 1: 写下失败的集成测试**

在 `tests/sql_completion.rs` 中 `alias_column_completion_uses_relation_columns_and_native_type` 之后追加：

```rust
#[test]
fn column_completion_detail_uses_short_type_spelling() {
    let mut entries = fixture();
    let connection = entries[0].id.profile_id();
    let table = entries[2].id.clone();
    for (name, native_type) in [
        ("code", "character varying(30)"),
        ("created_at", "timestamp without time zone"),
    ] {
        entries.push(
            CatalogEntry::relation_child(
                CatalogId::new(
                    connection,
                    CatalogKind::Column,
                    ["app", "public", "users", name],
                ),
                table.clone(),
                qualified("app", Some("public"), name),
                "column",
                OptionalMetadata::Unsupported,
                CatalogMetadata::Column(ColumnMetadata::new(2, native_type, true)),
            )
            .unwrap(),
        );
    }
    let index = CompletionIndex::new(&entries);
    let candidates = complete(
        "select c from users",
        8,
        SqlDialect::Postgres,
        &index,
        CompletionContext::default(),
    );

    let detail = |label: &str| {
        candidates
            .iter()
            .find(|candidate| candidate.label == label)
            .and_then(|candidate| candidate.detail.clone())
    };
    assert_eq!(detail("code").as_deref(), Some("varchar(30)"));
    assert_eq!(detail("created_at").as_deref(), Some("timestamp"));
}
```

`created_at` 走 `c` 前缀的 compact 匹配，与 `code` 同批返回；若断言取不到候选，先确认 fixture 的 relation 是否被 `select ... from users` 绑定，而不是放宽断言。

```bash
cargo test --test sql_completion column_completion_detail_uses_short_type_spelling -- --exact
```

预期失败：detail 仍为 `character varying(30)` 与 `timestamp without time zone`。

**Step 2: 在 `completion_detail` 中接入短拼写**

`src/sql/completion.rs`：

```rust
fn completion_detail(entry: &CatalogEntry) -> Option<String> {
    match &entry.metadata {
        CatalogMetadata::Column(column) => Some(super::short_type_name(&column.native_type)),
        CatalogMetadata::None | CatalogMetadata::Index(_) | CatalogMetadata::Constraint(_) => None,
    }
}
```

调用点（`src/sql/completion.rs:300`）保持 `completion_detail(entry).map(|detail| display_text(&detail))` 不变：短拼写作用于原始类型名，terminal 净化仍是展示路径的最后一步。

**Step 3: 在数据网格候选构造处接入短拼写**

`refresh_active_data_query_completion` 只有一个 `DataQueryCandidate` 构造点，改这一处即可同时覆盖 relation tab 与 SQL tab 两条列来源：

```rust
.filter_map(|(name, type_name)| {
    let quality = sql::identifier_match(&name, &prefix)?;
    Some((
        quality,
        DataQueryCandidate {
            name,
            type_name: type_name.as_deref().map(sql::short_type_name),
        },
    ))
})
```

不要去改上游三处列收集代码（`entry.metadata` 与两处 `result.columns`），保持单一接缝更容易验证，也避免误碰 `parse_relation_value` 依赖的语义类型。

**Step 4: 验证接缝改动**

```bash
cargo test --test sql_completion
cargo test --test ui_render relation_query_completion_is_anchored_to_active_input
```

`column_completion_detail_uses_short_type_spelling` 转绿；`alias_column_completion_uses_relation_columns_and_native_type` 的 `text<ESC>[31m` 与数据网格弹框的 `bigint<ESC>[31m` 断言必须仍然通过（两者都未命中别名表，原样透传）。

### Task 4: 用渲染测试锁定三列右对齐布局

**Files:**
- Modify: `tests/ui_render.rs:1420-1445`
- Test: `tests/ui_render.rs`

**Step 1: 给多候选场景加一个测试辅助函数**

在 `completion_app` 之后追加，避免每个测试重复拼 `CompletionCandidate`：

```rust
fn completion_app_with_details(sql: &str, replace: TextRange, rows: &[(&str, &str)]) -> App {
    let mut app = completion_app(sql, replace, rows[0].0);
    app.active_console_mut().completion = Some(CompletionPopup {
        candidates: rows
            .iter()
            .map(|(label, detail)| CompletionCandidate {
                label: (*label).into(),
                insert_text: (*label).into(),
                kind: CompletionKind::Column,
                detail: (!detail.is_empty()).then(|| (*detail).to_owned()),
                replace,
                score: CompletionScore {
                    context: 3,
                    name_match: 2,
                    schema: 1,
                },
            })
            .collect(),
        selected: 0,
    });
    app
}
```

**Step 2: 写下右对齐测试**

```rust
#[test]
fn completion_detail_column_is_right_aligned() {
    // The rows deliberately differ in `label + detail` width: a shared right
    // edge can only come from a real detail column, not from ragged text.
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        &[("id", "bigint"), ("sys_user", "varchar(200)")],
    );
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let short = find_ascii_cells(&buffer, popup.y + 1, "bigint").expect("short detail");
    let long = find_ascii_cells(&buffer, popup.y + 2, "varchar(200)").expect("long detail");
    let label = find_ascii_cells(&buffer, popup.y + 2, "sys_user").expect("long label");

    let short_end = short + "bigint".len() as u16;
    let long_end = long + "varchar(200)".len() as u16;
    assert_eq!(short_end, long_end, "detail column must be right aligned");
    // 右边框 1 格 + 行尾留白 1 格。
    assert_eq!(long_end, popup.right() - 2);
    // 最长 label 与类型列之间保留最小间距。
    assert!(long >= label + "sys_user".len() as u16 + 2);
}
```

**Step 3: 写下整行高亮测试**

```rust
#[test]
fn completion_selected_row_highlight_spans_the_popup_width() {
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        // The selected row is the narrow one, so a text-width highlight leaves
        // an obvious gap.
        &[("id", "bigint"), ("sys_user", "varchar(200)")],
    );
    let (buffer, state) = render_buffer_with_icons(&app, 120, 36, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();

    for x in popup.x + 1..popup.right() - 1 {
        assert_eq!(
            buffer[(x, popup.y + 1)].bg,
            Color::Rgb(99, 230, 216),
            "selected row must be a full-width bar at x={x}"
        );
    }
}
```

**Step 4: 写下窄视口降级测试**

```rust
#[test]
fn completion_detail_is_dropped_when_the_popup_cannot_fit_it() {
    let app = completion_app_with_details(
        "SELECT * FROM sys_u",
        TextRange::new(14, 19),
        &[("sys_user_created_at_index_name", "varchar(200)")],
    );
    // 56 格是弹框仍会渲染的最窄视口；此时内宽只够 icon 列与 label。
    let (buffer, state) = render_buffer_with_icons(&app, 56, 24, IconSet::new(IconMode::Ascii));
    let popup = state.completion_popup.unwrap();
    let label = find_ascii_cells(&buffer, popup.y + 1, "sys_user_created_at_index_name")
        .expect("full label");

    assert!(popup.right() <= 56);
    assert!(find_ascii_cells(&buffer, popup.y + 1, "varchar").is_none());
    // 类型列整列消失，而不是被边框裁成半截。
    for x in label + "sys_user_created_at_index_name".len() as u16..popup.right() - 1 {
        assert_eq!(
            buffer[(x, popup.y + 1)].symbol(),
            " ",
            "detail column must disappear entirely at x={x}"
        );
    }
}
```

视口宽度要按实测取：40 格与 48 格下 `state.completion_popup` 是 `None`（弹框根本不渲染），56 格是仍会渲染的最窄视口。此时实测 `popup.x = 17`、`width = 38`、内宽 36 格：`label_offset` 3 + label 30 + 行尾留白 1 之后只剩 2 格，不足 `COMPLETION_DETAIL_GAP` + `COMPLETION_DETAIL_MIN_CELLS`，detail 列收敛为 0。所以 label 必须取 30 格的 `sys_user_created_at_index_name`，25 格的名字挤不掉类型列。"detail 整列消失、label 完整"这两条不得放宽：改前的代码会在边框处留下半截 `v`，所以断言要覆盖 label 之后的每一格都是空白。

**Step 5: 运行并确认失败原因**

```bash
cargo test --test ui_render completion_detail_column_is_right_aligned -- --exact
cargo test --test ui_render completion_selected_row_highlight_spans_the_popup_width -- --exact
cargo test --test ui_render completion_detail_is_dropped_when_the_popup_cannot_fit_it -- --exact
```

预期失败：detail 紧跟 label（右端不齐、也不落在 `popup.right() - 2`）；选中行高亮只覆盖到该行文字末尾；窄视口下类型被边框裁成半截而不是整列隐藏。

### Task 5: 实现共享的三列布局并让两个弹框复用

**Files:**
- Modify: `src/ui/mod.rs:943-1054`（`render_completion_popup`）
- Modify: `src/ui/mod.rs:1094-1172`（`render_data_query_completion_popup`）
- Modify: `src/ui/mod.rs:4216`（`mod completion_popup_tests`）
- Test: `src/ui/mod.rs`、`tests/ui_render.rs`

**Step 1: 加入列宽模型**

在 `CompletionAnchor` 定义之后插入常量与 `CompletionColumns`：

```rust
const COMPLETION_DETAIL_GAP: u16 = 2;
const COMPLETION_ROW_RIGHT_PADDING: u16 = 1;
const COMPLETION_DETAIL_MAX_CELLS: u16 = 24;
const COMPLETION_DETAIL_MIN_CELLS: u16 = 4;

/// icon / label / detail 三列的列宽。`detail == 0` 表示不显示类型列。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CompletionColumns {
    icon: u16,
    label: u16,
    detail: u16,
}

impl CompletionColumns {
    fn measure<'a>(rows: impl Iterator<Item = (u16, &'a str, &'a str)>) -> Self {
        let mut columns = Self::default();
        for (icon, label, detail) in rows {
            columns.icon = columns.icon.max(icon);
            columns.label = columns.label.max(label.cell_width());
            columns.detail = columns.detail.max(detail.cell_width());
        }
        columns.detail = columns.detail.min(COMPLETION_DETAIL_MAX_CELLS);
        columns
    }

    /// label 列的起始偏移。`popup_x` 的锚点计算依赖它，必须保持 `icon + 1`。
    fn label_offset(self) -> u16 {
        self.icon.saturating_add(1)
    }

    fn content_width(self) -> u16 {
        self.label_offset()
            .saturating_add(self.label)
            .saturating_add(if self.detail == 0 {
                0
            } else {
                COMPLETION_DETAIL_GAP.saturating_add(self.detail)
            })
            .saturating_add(COMPLETION_ROW_RIGHT_PADDING)
            .max(4)
    }

    /// 用 clamp 后的实际内宽收敛列宽：label 优先，detail 吃剩余空间。
    fn fit(self, inner_width: u16) -> Self {
        let label = self.label.min(inner_width.saturating_sub(self.label_offset()));
        let detail = self.detail.min(
            inner_width
                .saturating_sub(self.label_offset())
                .saturating_sub(label)
                .saturating_sub(COMPLETION_DETAIL_GAP)
                .saturating_sub(COMPLETION_ROW_RIGHT_PADDING),
        );
        Self {
            label,
            detail: if detail >= COMPLETION_DETAIL_MIN_CELLS {
                detail
            } else {
                0
            },
            ..self
        }
    }
}
```

**Step 2: 加入共享的行拼装函数**

```rust
/// 拼一行候选：icon | label（左对齐补齐）| detail（在类型列内右对齐）| 整行背景补齐。
#[allow(clippy::too_many_arguments)]
fn completion_row(
    columns: CompletionColumns,
    inner_width: u16,
    icon: &str,
    label_spans: Vec<Span<'static>>,
    label_cells: u16,
    detail: &str,
    row_style: Style,
    detail_style: Style,
) -> ListItem<'static> {
    let icon_padding = " ".repeat(usize::from(columns.icon.saturating_sub(icon.cell_width())));
    let mut spans = Vec::with_capacity(label_spans.len() + 3);
    spans.push(Span::styled(format!("{icon_padding}{icon} "), row_style));
    spans.extend(label_spans);
    let mut used = columns.label_offset().saturating_add(label_cells);
    if columns.detail > 0 && !detail.is_empty() {
        let detail = truncate_to_cell_width(detail, columns.detail);
        let detail_cells = detail.as_str().cell_width();
        let padding = columns
            .label
            .saturating_sub(label_cells)
            .saturating_add(COMPLETION_DETAIL_GAP)
            .saturating_add(columns.detail.saturating_sub(detail_cells));
        spans.push(Span::styled(" ".repeat(usize::from(padding)), row_style));
        spans.push(Span::styled(detail, detail_style));
        used = used.saturating_add(padding).saturating_add(detail_cells);
    }
    let trailing = inner_width.saturating_sub(used);
    if trailing > 0 {
        spans.push(Span::styled(" ".repeat(usize::from(trailing)), row_style));
    }
    ListItem::new(Line::from(spans))
}
```

行尾补齐是选中行高亮变成完整矩形的唯一来源；非选中行填的是 `theme.surface_raised`，与 block 背景一致，视觉无变化。

**Step 3: 改写 `render_completion_popup` 的宽度计算**

用 `CompletionColumns` 替换 `icon_width`/`label_offset`/`content_width` 三段（`src/ui/mod.rs:969-999`）：

```rust
    let columns = CompletionColumns::measure(popup.candidates.iter().map(|candidate| {
        (
            icons.completion(candidate.kind).cell_width(),
            candidate.label.as_str(),
            candidate.detail.as_deref().unwrap_or(""),
        )
    }));
    let visible_rows = popup.candidates.len().min(10) as u16;
    let desired_width = columns.content_width().saturating_add(POPUP_BORDER_WIDTH);
    let desired_height = visible_rows.saturating_add(POPUP_BORDER_HEIGHT);
    let popup_x = anchor
        .replacement_start_x
        .map(|label_x| label_x.saturating_sub(columns.label_offset().saturating_add(1)))
        .unwrap_or(anchor.cursor.x);
```

列宽按全部候选度量（不只是可见行），这样弹框被裁高时列宽不会跳动。`popup_x` 的表达式除了取值来源改成 `columns.label_offset()` 外保持不变。

**Step 4: 改写 `render_completion_popup` 的行渲染**

`state.completion_popup = Some(area);` 之后先按实际内宽收敛列宽，再拼行：

```rust
    let inner_width = area.width.saturating_sub(POPUP_BORDER_WIDTH);
    let columns = columns.fit(inner_width);
    let editor_text = app.active_editor_text().ok();
    let items = popup
        .candidates
        .iter()
        .take(usize::from(area.height.saturating_sub(POPUP_BORDER_HEIGHT)).min(10))
        .enumerate()
        .map(|(index, candidate)| {
            let row_style = if index == popup.selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            let match_query = editor_text
                .as_deref()
                .and_then(|text| text.get(candidate.replace.start..candidate.replace.end));
            let label_spans = match_query.map_or_else(
                || vec![Span::styled(candidate.label.clone(), row_style)],
                |query| completion_label_spans(&candidate.label, query, row_style),
            );
            completion_row(
                columns,
                inner_width,
                icons.completion(candidate.kind),
                label_spans,
                candidate.label.as_str().cell_width(),
                candidate.detail.as_deref().unwrap_or(""),
                row_style,
                row_style.fg(if index == popup.selected {
                    theme.background
                } else {
                    theme.muted
                }),
            )
        })
        .collect::<Vec<_>>();
```

`completion_label_spans` 与选中行的前景色规则保持原样，不要在本任务里改高亮配色。

**Step 5: 让数据网格弹框复用同一套布局**

`render_data_query_completion_popup` 里先把净化后的 `(name, detail)` 收成一次性的 `rows`（现在的实现在宽度计算和行渲染里各净化一遍），再走同样的列模型：

```rust
    const DATA_QUERY_ICON: &str = "CL";

    let rows = completion
        .candidates
        .iter()
        .map(|candidate| {
            (
                crate::security::sanitize_terminal_text(&candidate.name),
                candidate
                    .type_name
                    .as_deref()
                    .map(crate::security::sanitize_terminal_text)
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let columns = CompletionColumns::measure(rows.iter().map(|(name, detail)| {
        (
            DATA_QUERY_ICON.cell_width(),
            name.as_str(),
            detail.as_str(),
        )
    }));
    let visible_rows = rows.len().min(10) as u16;
    let desired_width = columns.content_width().saturating_add(POPUP_BORDER_WIDTH);
```

行渲染：

```rust
    let inner_width = area.width.saturating_sub(POPUP_BORDER_WIDTH);
    let columns = columns.fit(inner_width);
    let items = rows
        .iter()
        .take(usize::from(area.height.saturating_sub(POPUP_BORDER_HEIGHT)).min(10))
        .enumerate()
        .map(|(index, (name, detail))| {
            let selected = index == completion.selected;
            let row_style = if selected {
                Style::new().fg(theme.background).bg(theme.accent)
            } else {
                Style::new().fg(theme.text).bg(theme.surface_raised)
            };
            completion_row(
                columns,
                inner_width,
                DATA_QUERY_ICON,
                vec![Span::styled(name.clone(), row_style)],
                name.as_str().cell_width(),
                detail,
                row_style,
                row_style.fg(if selected {
                    theme.background
                } else {
                    theme.muted
                }),
            )
        })
        .collect::<Vec<_>>();
```

`completion_row` 会补上 icon 后的空格，渲染出的前缀仍是 `"CL "`，`relation_data_query_completion_popup_*` 的现有断言不受影响。

**Step 6: 给列宽模型补单测**

在 `mod completion_popup_tests`（`src/ui/mod.rs:4216`）中追加：

```rust
    #[test]
    fn completion_columns_prefer_labels_over_details_when_space_is_tight() {
        let columns = CompletionColumns::measure(
            [(2u16, "create_time", "timestamp"), (2, "id", "bigint")].into_iter(),
        );

        assert_eq!(columns.label_offset(), 3);
        assert_eq!(columns.label, 11);
        assert_eq!(columns.detail, 9);
        assert_eq!(columns.content_width(), 26);
        assert_eq!(columns.fit(26), columns);

        let clipped = columns.fit(24);
        assert_eq!(clipped.label, 11);
        assert_eq!(clipped.detail, 7);

        let tight = columns.fit(20);
        assert_eq!(tight.label, 11);
        assert_eq!(tight.detail, 0);
    }

    #[test]
    fn completion_columns_cap_overlong_details() {
        let columns = CompletionColumns::measure(
            [(2u16, "code", "a_very_long_user_defined_type_name")].into_iter(),
        );

        assert_eq!(columns.detail, COMPLETION_DETAIL_MAX_CELLS);
    }
```

**Step 7: 跑通渲染测试**

```bash
cargo test --test ui_render
cargo test --lib ui::completion_popup_tests
```

Task 4 的三个测试转绿，并且这些既有测试必须继续通过：`completion_candidate_label_aligns_with_identifier_start`、`completion_candidate_labels_share_a_fixed_icon_column`、`completion_popup_stays_fixed_while_typing`、`completion_popup_keeps_origin_when_candidate_width_changes`、`completion_label_alignment_handles_multiline_tabs_and_wide_characters`、`completion_candidate_label_highlight_preserves_selected_row_contrast`、`completion_popup_stays_in_editor_when_identifier_starts_at_column_zero`。若锚点类测试失败，第一嫌疑是 `label_offset()` 被改动或 `popup_x` 用了 `fit` 之后的列宽。

### Task 6: 文档与全量验证

**Files:**
- Modify: `docs/architecture.md:187-191`
- Test: 全量测试套件

**Step 1: 补充展示层约束**

`docs/architecture.md` 现有段落说明"补全 label/detail 仅为展示做净化，原始插入/请求值分离"。在该段末尾追加一句，把短拼写纳入同一条不变量：

```markdown
Completion detail column types are additionally normalized to compact display
spellings; catalog `native_type` and result-set column types keep their original
text because cell-edit value parsing depends on them.
```

**Step 2: 全量验证**

```bash
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

三条命令与 CI（`.github/workflows/ci.yml:34-36`）一致。需要真实数据库的适配器测试不在本轮范围内（本改动不触碰任何 adapter）。

**Step 3: 手工验收**

连接 Postgres，在 SQL Editor 中输入 `select ` 后触发 `sys_user` 的列补全，确认：

- 类型显示为 `varchar(30)` / `varchar(200)` / `timestamp` / `bigint`。
- 所有类型右端对齐，距右边框 1 格。
- 选中行高亮是完整矩形。
- 弹框宽度明显小于改动前（不再被 `character varying` 撑开）。
- 把终端宽度压到 40 列左右，确认类型列先截断再整列消失，label 始终完整。

再切到数据网格的 WHERE 输入框触发列补全，确认同样的对齐与短拼写。

## 实施记录：与计划的偏差

按 Task 1–6 逐项实施后，以下六处与初版计划不同，原因均已在对应 Task 正文中就地更正：

1. `split_time_zone` 改用 `checked_sub` + `value.get(split..)`。`split_at(value.len() - phrase.len())` 的偏移是字节数，以多字节字符结尾的类型名会让它落在字符内部并 panic；新增 `abc中xxxxxxxxxxxxx` 作为回归输入钉住这一点。
2. 匹配到时区短语时把 base 转小写。原函数体会让 `TIMESTAMP WITHOUT TIME ZONE` 输出 `TIMESTAMP`，直接违反 Task 1 自己的断言，`with time zone` 还会拼出 `TIMESTAMPtz`。
3. 右对齐测试换用 `[("id", "bigint"), ("sys_user", "varchar(200)")]`。原数据两行 `label + detail` 宽度相等（8+12 = 11+9），改动前的参差渲染也能让右端对齐，测试在实现前就是绿的，不构成约束。整行高亮测试沿用同一组数据，让选中行成为窄行，改动前的失败从 1 格扩大到 13 格。
4. 窄视口测试改为 56×24 视口 + 30 格 label。实测 40 格与 48 格下 `state.completion_popup` 是 `None`（弹框不渲染），原测试无法成立；56 格下内宽 36 格，25 格的 label 挤不掉类型列。断言同时要求 label 之后每一格为空白，因为改动前会在边框处留下半截 `v`。
5. `completion_row` 不接收 `label_cells` 参数，改为在函数内部按 `label_spans` 累加 `cell_width()`。少一个必须与 spans 保持同步的入参，调用方无法传错。
6. `tests/ui_render.rs::sql_result_query_completion_is_rendered_above_the_grid` 的断言从 `BOOLEAN` 改为 `active  bool`。这是短拼写的预期结果而非回归，改断言而不是改实现。

Task 4 的验证命令中 `cargo test --test ui_render relation_data_query_completion` 匹配不到测试，实际名称是 `relation_query_completion_is_anchored_to_active_input`，正文已更正。

全量验证结果：`cargo fmt --all -- --check` 与 `cargo clippy --all-targets --all-features -- -D warnings` 干净；lib 446 项、`tests/ui_render.rs` 130 项、`tests/sql_completion.rs` 42 项全绿。`tests/keymap.rs` 的 `insert_mode_preserves_printable_characters` 与 `normal_mode_global_keys_win_over_editor_and_completion` 两项失败属于基线 `0796fda`（把该 commit 的纯净树导出到临时目录单独构建，得到逐字一致的失败输出），与本改动无关。回归源头是 `86e0fc8 feat(config): migrate more keybindings`：它把 `focus-next-pane` / `focus-previous-pane` 的配置匹配提到 `Keymap::map` 最前面，insert 模式下 Tab 因此先命中 `FocusNext` 而不再透传给编辑器；同时它把 normal 模式分支里字面量 `KeyCode::BackTab` 换成 `Shift-Tab` 的配置匹配，而 `KeyCode::BackTab` 事件不带 SHIFT 修饰，于是落到编辑器透传返回 `EditorKey(BackTab)`。本改动未触碰输入层，`src/input/keymap.rs` 只读 `completion.is_some()`，看不到 detail 内容。
