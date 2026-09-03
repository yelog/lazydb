# Workspace Tab Overflow Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在 workspace tab 超出终端宽度时显示左右导航箭头，并保证鼠标点击、现有快捷键切换、workspace 恢复和终端 resize 后当前 tab 始终完整可见。

**Architecture:** 将 tab bar 改成按完整 tab 单元计算的无状态可视窗口，而不是对整行文本做字符偏移裁剪。渲染函数每帧根据 tab 宽度、`active_tab` 和可用宽度计算 `[start, end)`，仅渲染该范围并注册命中区域；左右箭头携带相邻隐藏 tab 的索引，鼠标点击复用现有 `Action::ActivateTab`。不向 `App`、workspace 持久化结构或 `UiState` 增加滚动偏移，从而让所有改变 `active_tab` 的入口自然获得自动跟随行为。

**Tech Stack:** Rust 2024、Ratatui 0.30、Crossterm 0.29、`unicode-width`、现有 `Action`/`HitTarget`/`UiState` 输入与渲染架构。

---

## Design Decisions

1. 可视窗口只能在 tab 边界切分，不允许正常情况下显示半个 tab。
2. 总宽度可容纳全部 tab 时不显示箭头，保持现有视觉效果。
3. 发生溢出时两侧各保留一个固定宽度的箭头区域；没有对应方向的隐藏 tab 时显示 disabled 箭头但不注册鼠标命中区域，避免内容区域左右跳动。
4. 点击左箭头激活 `start - 1`，点击右箭头激活 `end`。箭头不是独立滚动状态，点击后由 `Action::ActivateTab` 改变当前 tab，下一帧自动计算新窗口。
5. `NextTab`、`PreviousTab`、`ActivateTab`、新建、关闭、workspace 恢复和 resize 均不增加专用同步代码；它们只要改变 `active_tab`，渲染层就保证活动项可见。
6. 第一版不实现“手动浏览但不激活”的 tab strip，因为它会允许当前 tab 被隐藏，并需要新增跨帧滚动状态，不符合本需求的核心约束。
7. 单个 tab 比内容区还宽时允许在该 tab 内部截断，但必须优先保留图标、最少标题内容和关闭按钮；命中区域只能覆盖实际渲染区域。
8. 所有布局宽度使用 terminal cell width，禁止使用字节长度或 `chars().count()` 作为显示宽度。

## Target Behavior

假设可视窗口是 `[start, end)`：

- `start == 0` 时左箭头 disabled；否则左箭头激活 `start - 1`。
- `end == tabs.len()` 时右箭头 disabled；否则右箭头激活 `end`。
- 必须始终满足 `start <= active_tab < end`。
- 窗口优先从活动 tab 开始向右容纳完整 tab，再用剩余空间向左扩展。
- 该确定性规则不依赖上一帧，因此首帧恢复、测试辅助函数和 resize 行为一致。
- 从最后一个 tab 通过快捷键循环到第一个 tab 时窗口回到左端；反向循环时窗口落到右端。

## Out Of Scope

- 不新增或修改 tab 切换快捷键。
- 不修改 workspace 文件版本和持久化 schema。
- 不实现鼠标滚轮横向滚动。
- 不实现拖拽排序 tab。
- 不改变 tab 标题的现有最大 48 字符规则，除非单个 tab 超过 viewport。
- 不修改 `NextTab`、`PreviousTab` 的循环切换语义。

### Task 1: Add Tab Viewport Calculation

**Files:**
- Modify: `src/ui/mod.rs`，在 `render_tabs` 前增加内部布局类型和纯函数
- Test: `src/ui/mod.rs` 的现有 `#[cfg(test)] mod tests`

**Step 1: 写可容纳全部 tab 的失败测试**

增加只使用宽度数组的纯函数测试：

```rust
#[test]
fn tab_viewport_uses_full_width_without_overflow_controls() {
    let viewport = tab_viewport(&[8, 10, 12], 1, 30);

    assert_eq!(viewport.start, 0);
    assert_eq!(viewport.end, 3);
    assert!(!viewport.overflowed);
}
```

`TabViewport.end` 使用 exclusive index，`overflowed` 表示是否需要保留左右箭头槽位。

**Step 2: 运行测试并确认失败**

Run: `cargo test --lib tab_viewport_uses_full_width_without_overflow_controls`

Expected: FAIL，错误为 `tab_viewport` 或 `TabViewport` 尚未定义。

**Step 3: 增加最小类型和无溢出分支**

在 `render_tabs` 前增加：

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TabViewport {
    start: usize,
    end: usize,
    overflowed: bool,
}

fn tab_viewport(widths: &[u16], active: usize, area_width: u16) -> TabViewport {
    let total = widths
        .iter()
        .copied()
        .fold(0_u16, u16::saturating_add);
    if total <= area_width {
        return TabViewport {
            start: 0,
            end: widths.len(),
            overflowed: false,
        };
    }

    todo!("calculate overflow window")
}
```

**Step 4: 运行测试并确认通过**

Run: `cargo test --lib tab_viewport_uses_full_width_without_overflow_controls`

Expected: PASS。

**Step 5: 写活动 tab 位于不同位置的失败测试**

使用固定的两个 cell 作为箭头预算，增加：

```rust
#[test]
fn tab_viewport_keeps_first_active_tab_visible() {
    let viewport = tab_viewport(&[8, 8, 8, 8], 0, 20);
    assert_eq!(viewport, TabViewport { start: 0, end: 2, overflowed: true });
}

#[test]
fn tab_viewport_keeps_middle_active_tab_visible() {
    let viewport = tab_viewport(&[8, 8, 8, 8], 2, 20);
    assert_eq!(viewport, TabViewport { start: 1, end: 3, overflowed: true });
}

#[test]
fn tab_viewport_keeps_last_active_tab_visible() {
    let viewport = tab_viewport(&[8, 8, 8, 8], 3, 20);
    assert_eq!(viewport, TabViewport { start: 2, end: 4, overflowed: true });
}
```

测试期望表达以下规则：20 cell 中 2 cell 给箭头，18 cell 可以容纳两个 8-cell tab；活动项在末端时先包含活动项，再向左填满剩余空间。

**Step 6: 运行测试并确认失败**

Run: `cargo test --lib tab_viewport_keeps_`

Expected: FAIL，当前溢出分支仍为 `todo!()`。

**Step 7: 实现完整 tab 窗口算法**

实现时遵循以下伪代码，不引入跨帧状态：

```rust
const TAB_OVERFLOW_CONTROLS_WIDTH: u16 = 2;

fn tab_viewport(widths: &[u16], active: usize, area_width: u16) -> TabViewport {
    if widths.is_empty() {
        return TabViewport { start: 0, end: 0, overflowed: false };
    }

    let active = active.min(widths.len() - 1);
    let total = saturating_sum(widths);
    if total <= area_width {
        return TabViewport { start: 0, end: widths.len(), overflowed: false };
    }

    let available = area_width.saturating_sub(TAB_OVERFLOW_CONTROLS_WIDTH);
    let mut start = active;
    let mut end = active + 1;
    let mut used = widths[active].min(available);

    while end < widths.len() && used.saturating_add(widths[end]) <= available {
        used = used.saturating_add(widths[end]);
        end += 1;
    }
    while start > 0 && used.saturating_add(widths[start - 1]) <= available {
        start -= 1;
        used = used.saturating_add(widths[start]);
    }

    TabViewport { start, end, overflowed: true }
}
```

如果实际测试表明“先向右再向左”让中间活动项偏左，可调整为左右交替扩展，但不要引入 `UiState` offset。验收约束只要求确定性、完整 tab 和活动项可见。

**Step 8: 增加边界测试**

覆盖：

```rust
#[test]
fn tab_viewport_handles_empty_tabs_and_zero_width() { /* empty + width 0 */ }

#[test]
fn tab_viewport_includes_oversized_active_tab() { /* one width 40, area 12 */ }

#[test]
fn tab_viewport_clamps_out_of_range_active_index() { /* active > len */ }

#[test]
fn tab_viewport_treats_exact_fit_as_not_overflowed() { /* sum == area */ }
```

对多个宽度和所有合法 active index 增加循环断言：

```rust
assert!(viewport.start <= active);
assert!(active < viewport.end);
assert!(viewport.start < viewport.end);
assert!(viewport.end <= widths.len());
```

**Step 9: 运行纯函数测试**

Run: `cargo test --lib tab_viewport`

Expected: 所有 `tab_viewport` 测试 PASS，无 panic。

### Task 2: Add Overflow Arrow Icons

**Files:**
- Modify: `src/ui/icons.rs:43-120`
- Test: `src/ui/icons.rs` 的现有测试模块

**Step 1: 写三种 icon mode 的失败测试**

```rust
#[test]
fn tab_navigation_icons_have_single_cell_fallbacks() {
    assert_eq!(IconSet::new(IconMode::Ascii).tab_previous(), "<");
    assert_eq!(IconSet::new(IconMode::Ascii).tab_next(), ">");
    assert_eq!(IconSet::new(IconMode::Unicode).tab_previous(), "‹");
    assert_eq!(IconSet::new(IconMode::Unicode).tab_next(), "›");
    assert_eq!(IconSet::new(IconMode::NerdFont).tab_previous(), "‹");
    assert_eq!(IconSet::new(IconMode::NerdFont).tab_next(), "›");
}
```

同时用项目现有的 cell width trait/assertion确认每个返回值恰好占 1 cell。

**Step 2: 运行测试并确认失败**

Run: `cargo test --lib tab_navigation_icons_have_single_cell_fallbacks`

Expected: FAIL，`tab_previous` 和 `tab_next` 尚未定义。

**Step 3: 实现图标方法**

在 `IconSet` 中增加：

```rust
pub const fn tab_previous(self) -> &'static str {
    match self.mode {
        IconMode::Ascii => "<",
        IconMode::NerdFont | IconMode::Unicode => "‹",
    }
}

pub const fn tab_next(self) -> &'static str {
    match self.mode {
        IconMode::Ascii => ">",
        IconMode::NerdFont | IconMode::Unicode => "›",
    }
}
```

不要引用新的 Nerd Font 常量；这两个 Unicode 字符在支持 Unicode 的模式下更稳定。

**Step 4: 运行测试并确认通过**

Run: `cargo test --lib tab_navigation_icons_have_single_cell_fallbacks`

Expected: PASS。

### Task 3: Render Only the Visible Tab Window

**Files:**
- Modify: `src/ui/mod.rs:1262-1356`
- Test: `tests/ui_render.rs`

**Step 1: 增加窄终端 tab fixture**

在 `tests/ui_render.rs` 增加辅助函数，使用 SQL console 构建稳定且无需数据库 I/O 的 tab 集合：

```rust
fn app_with_named_tabs(names: &[&str], active: usize) -> App {
    let mut app = App::new(Vec::new());
    app.tabs = names
        .iter()
        .map(|name| WorkspaceTab::Sql(lazydb::model::tab::ConsoleTab::new(*name)))
        .collect();
    app.active_tab = active;
    app
}
```

注意默认 console 的关闭规则依赖名称 `console`；测试名称应使用唯一且可关闭的名称，例如 `alpha-unique`、`bravo-unique`。

**Step 2: 写活动尾部 tab 可见的失败渲染测试**

```rust
#[test]
fn overflowing_workspace_tabs_show_the_active_tail_tab() {
    let app = app_with_named_tabs(
        &["alpha-unique", "bravo-unique", "charlie-unique", "delta-active"],
        3,
    );

    let output = render(&app, 56, 20);

    assert!(output.contains("delta-active"), "{output}");
    assert!(output.contains("‹"), "{output}");
}
```

测试宽度必须先确认 layout 仍显示 workspace tabs；项目最小终端宽度是 56。

**Step 3: 运行测试并确认失败**

Run: `cargo test --test ui_render overflowing_workspace_tabs_show_the_active_tail_tab -- --nocapture`

Expected: FAIL，当前 `Paragraph` 从首个 tab 开始裁剪，`delta-active` 不可见。

**Step 4: 在 `render_tabs` 中构建 tab 展示单元**

保持现有标题清洗、48 字符限制、图标选择和默认 console 不可关闭规则，但先将每项收集为局部结构：

```rust
struct RenderedTab {
    index: usize,
    id: Uuid,
    label: String,
    close: Option<String>,
    width: u16,
}
```

具体要求：

- `label_width`、`close_width` 和 `width` 使用 `cell_width()`。
- 将 `usize` cell width 安全转换为 `u16`，使用 `u16::try_from(...).unwrap_or(u16::MAX)` 或饱和 helper。
- 不重复调用标题清洗或图标选择。
- 保留 active/inactive 现有颜色与 bold 样式。

如果局部结构让函数过长，可将“生成单个 tab 展示数据”提取为一个私有函数；不要将业务 tab 状态复制到新的长期模型中。

**Step 5: 根据 viewport 分配固定箭头槽和内容区**

伪代码：

```rust
let widths = rendered_tabs.iter().map(|tab| tab.width).collect::<Vec<_>>();
let viewport = tab_viewport(&widths, app.active_tab, area.width);
let (left_area, tabs_area, right_area) = if viewport.overflowed {
    (
        Rect::new(area.x, area.y, 1, area.height.min(1)),
        Rect::new(area.x.saturating_add(1), area.y, area.width.saturating_sub(2), area.height),
        Rect::new(area.right().saturating_sub(1), area.y, 1, area.height.min(1)),
    )
} else {
    (Rect::default(), area, Rect::default())
};
```

实际实现优先使用 Ratatui `Layout` 或一个小 helper，确保 `area.width` 为 0/1 时不出现越界 Rect。箭头只占一行，因为 tab bar 本身为一行。

**Step 6: 只渲染 `[start, end)` 并注册真实命中区域**

改写循环：

```rust
for tab in &rendered_tabs[viewport.start..viewport.end] {
    // push label/close span
    // HitTarget::Tab(tab.index)
    // HitTarget::CloseTab(tab.id)
}
```

命中区域约束：

- `x` 从 `tabs_area.x` 开始，而不是 `area.x`。
- 仅当 `x < tabs_area.right()` 且计算后的宽度大于 0 时 push region。
- 每个 region 必须完全位于 `tabs_area` 内。
- 不可见 tab 不得注册 `Tab` 或 `CloseTab`。
- 关闭按钮只能使用实际剩余宽度，不能覆盖右箭头。

**Step 7: 处理单个超宽活动 tab**

增加私有 helper 按 cell 截断标题，而不是按字节截断：

```rust
fn truncate_to_cell_width(value: &str, max_width: u16) -> String
```

规则：

- 迭代 `char`，累加每个字符的 terminal cell width。
- 不拆分宽字符。
- 如果发生截断且至少有一个 cell 可用，末尾使用单 cell `…`；ASCII icon mode 可仍使用 `…`，因为项目 UI 已支持 Unicode；若项目对纯 ASCII 输出有严格要求则改为 `~`。
- 对超宽 tab 重新分配：固定保留图标外围、关闭按钮和最少一个标题 cell，其余给标题。
- `RenderedTab.width` 更新为实际渲染宽度。

如果当前最小终端宽度和 48 字符上限已经保证单个 tab 总能被截断，应仍保留该 helper 的单元测试，防止长宽字符标题破坏布局。

**Step 8: 运行尾部活动 tab 测试**

Run: `cargo test --test ui_render overflowing_workspace_tabs_show_the_active_tail_tab -- --nocapture`

Expected: PASS。

**Step 9: 增加无溢出和活动首部测试**

```rust
#[test]
fn workspace_tab_arrows_are_hidden_when_all_tabs_fit() { /* assert no ‹/› */ }

#[test]
fn overflowing_workspace_tabs_show_the_active_head_tab() { /* active 0 visible + › */ }

#[test]
fn overflowing_workspace_tabs_keep_a_middle_active_tab_visible() { /* active middle */ }
```

为避免数据库图标或其他页面文本造成箭头字符误判，箭头断言应同时检查对应 `HitTarget`，最终在 Task 4 引入语义化目标后替换纯文本断言。

**Step 10: 运行相关渲染测试**

Run: `cargo test --test ui_render workspace_tab`

Expected: PASS。

### Task 4: Add Semantic Arrow Hit Targets

**Files:**
- Modify: `src/ui/mod.rs:136-186`
- Modify: `src/ui/mod.rs:1262-1356`
- Modify: `src/input/mouse.rs:72-89`
- Test: `tests/mouse.rs`

**Step 1: 写箭头鼠标映射的失败测试**

在 `tests/mouse.rs` 构建多个长标题 tab，使用 56x20 backend 和持久 `UiState` 渲染活动首项。断言右箭头目标存在且点击映射到第一个隐藏 tab：

```rust
let target = HitTarget::TabScrollRight(first_hidden_index);
assert_click_maps(&state, &app, &target, Action::ActivateTab(first_hidden_index));
```

再将活动项改为最后一项、复用同一 `UiState` 重绘，断言：

```rust
let target = HitTarget::TabScrollLeft(last_hidden_index);
assert_click_maps(&state, &app, &target, Action::ActivateTab(last_hidden_index));
```

**Step 2: 运行测试并确认失败**

Run: `cargo test --test mouse workspace_tab_overflow_arrows_activate_hidden_tabs -- --nocapture`

Expected: FAIL，`HitTarget` 尚无对应变体。

**Step 3: 增加 `HitTarget` 变体**

在 `src/ui/mod.rs` 增加：

```rust
TabScrollLeft(usize),
TabScrollRight(usize),
```

索引是点击后应激活的 tab，而不是滚动页码或像素/cell offset。

**Step 4: 渲染箭头和命中区域**

当 `viewport.overflowed`：

- 始终绘制左右箭头槽，保持 tab 内容区位置稳定。
- `viewport.start > 0` 时左箭头使用 enabled style，并注册 `TabScrollLeft(viewport.start - 1)`。
- `viewport.start == 0` 时左箭头使用 `theme.border` 或 `theme.muted` 的弱化样式，不注册 hit region。
- `viewport.end < rendered_tabs.len()` 时右箭头注册 `TabScrollRight(viewport.end)`。
- 到达右端时右箭头 disabled 且不注册 hit region。
- enabled 箭头建议使用 `theme.action`，disabled 箭头使用 `theme.border`，背景沿用 `theme.background`。

不要让 disabled 箭头映射到当前 tab，也不要循环；tab 循环仍由现有键盘动作负责。

**Step 5: 映射鼠标目标到已有 Action**

在 `src/input/mouse.rs` 的左键 match 中增加：

```rust
HitTarget::TabScrollLeft(index) | HitTarget::TabScrollRight(index) => {
    Some(Action::ActivateTab(index))
}
```

不修改 `src/action.rs` 和 `App::update`。

同时更新 `src/input/mouse.rs` 中枚举 `HitTarget` 的 exhaustive match（例如鼠标 cursor/drag 分类），将两个新目标归入普通 clickable target。

**Step 6: 运行箭头鼠标测试**

Run: `cargo test --test mouse workspace_tab_overflow_arrows_activate_hidden_tabs -- --nocapture`

Expected: PASS。

**Step 7: 写不可见 tab 无命中区域的失败测试**

```rust
#[test]
fn workspace_tab_overflow_only_registers_visible_tab_hit_regions() {
    // active first; force trailing tabs outside viewport
    // collect HitTarget::Tab indices
    // assert active/visible indices exist
    // assert trailing hidden index does not exist
    // assert every tab-related HitRegion has width > 0 and is inside tab bar
}
```

为检查边界，可以从已知 layout 或箭头 region 的 `y`/左右边界推导 tab bar；不要硬编码与 `AppLayout` 无关的坐标。

**Step 8: 修正所有零宽/越界命中区域**

若测试失败，确保 `render_tabs` 只在裁剪后宽度大于 0 时 push `HitRegion`，并使用 `tabs_area.right()` 作为上界。

**Step 9: 运行 mouse 测试集**

Run: `cargo test --test mouse`

Expected: 全部 PASS。

### Task 5: Verify Automatic Following Across State Changes

**Files:**
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`
- No production changes expected unless tests reveal a defect

**Step 1: 写现有快捷键动作切换后的重绘测试**

使用同一个 `UiState`，避免测试只覆盖临时 state：

```rust
#[test]
fn workspace_tab_viewport_follows_keyboard_tab_actions() {
    let mut app = app_with_named_tabs(/* enough long names to overflow */, 0);
    let mut state = UiState::new();
    render_into_existing_state(&app, 56, 20, &mut state);

    for _ in 0..app.tabs.len() - 1 {
        app.update(Action::NextTab);
    }
    let output = render_into_existing_state(&app, 56, 20, &mut state);

    assert!(output.contains(app.tabs[app.active_tab].title()));
    assert!(state.hit_regions.iter().any(|region| {
        region.target == HitTarget::Tab(app.active_tab)
    }));
}
```

这里直接发 `Action::NextTab`，因为 `tests/keymap.rs:610-642` 已经覆盖快捷键到动作的映射。

**Step 2: 运行测试并确认通过**

Run: `cargo test --test ui_render workspace_tab_viewport_follows_keyboard_tab_actions -- --nocapture`

Expected: PASS。若失败，修复渲染窗口，不要在 `App::update` 中加入 UI offset。

**Step 3: 增加循环切换测试**

覆盖：

- 最后一个 tab 执行 `NextTab` 后，第一个 tab 可见。
- 第一个 tab 执行 `PreviousTab` 后，最后一个 tab 可见。

Run: `cargo test --test ui_render workspace_tab_viewport_follows_wraparound -- --nocapture`

Expected: PASS。

**Step 4: 增加关闭活动 tab 后自动跟随测试**

使用不涉及 transaction confirmation 的 relation tab 或空闲 SQL tab：

- 激活尾部 tab并渲染。
- 执行 `CloseActiveTab`。
- 使用同一个 `UiState` 重绘。
- 断言新的 `active_tab` 标题和 `HitTarget::Tab(active_tab)` 可见。

Run: `cargo test --test ui_render workspace_tab_viewport_follows_active_tab_close -- --nocapture`

Expected: PASS。

**Step 5: 增加 resize 测试**

- 宽终端渲染活动尾部 tab。
- 用窄终端 backend 和同一个 `UiState` 重绘。
- 断言活动 tab 可见、箭头出现、命中区域在新边界内。
- 再放宽终端，断言所有 tab 可见且箭头 hit targets 消失。

Run: `cargo test --test ui_render workspace_tab_viewport_recalculates_after_resize -- --nocapture`

Expected: PASS。

**Step 6: 增加不同 icon mode 的布局测试**

调用 `render_with_state_using_icons`，分别使用：

```rust
IconSet::new(IconMode::Ascii)
IconSet::new(IconMode::Unicode)
IconSet::new(IconMode::NerdFont)
```

对每种模式断言：

- 活动 tab 有 `HitTarget::Tab(active)`。
- enabled 箭头目标存在。
- 所有 tab/close/arrow hit region 宽度大于 0。

Run: `cargo test --test ui_render workspace_tab_overflow_supports_all_icon_modes -- --nocapture`

Expected: PASS。

### Task 6: Regression Verification and Cleanup

**Files:**
- Modify if needed: `src/ui/mod.rs`
- Modify if needed: `src/ui/icons.rs`
- Modify if needed: `src/input/mouse.rs`
- Test: `src/ui/mod.rs`
- Test: `src/ui/icons.rs`
- Test: `tests/ui_render.rs`
- Test: `tests/mouse.rs`

**Step 1: 格式化改动**

Run: `cargo fmt --all -- --check`

Expected: PASS。若失败，运行 `cargo fmt --all`，再重复 check。

**Step 2: 运行定向测试**

Run: `cargo test --lib tab_viewport`

Expected: PASS。

Run: `cargo test --test ui_render workspace_tab -- --nocapture`

Expected: PASS。

Run: `cargo test --test mouse`

Expected: PASS。

Run: `cargo test --test keymap maps_tab_sequences_from_editor_normal_mode`

Expected: PASS，确认现有 `Ctrl+PageUp/PageDown`、`[t`、`]t`、`gt` 和 `gT` 行为未回归。

Run: `cargo test --test workspace_tabs mixed_tabs_cycle_and_activate_without_sql_assumptions`

Expected: PASS，确认业务层循环逻辑未改变。

**Step 3: 运行 Clippy**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: PASS，无新增 warning。特别检查 `usize` 到 `u16` 的转换、手写 range loop 和饱和算术告警。

**Step 4: 运行完整测试集**

Run: `cargo test --all-targets --all-features`

Expected: PASS。

**Step 5: 人工 TUI 验收**

Run: `cargo run -- --icons unicode`

手工步骤：

1. 连续新建足够多的 SQL console，直到 tab bar 溢出。
2. 确认左右箭头槽出现，当前 tab 完整可见。
3. 点击左、右箭头，确认激活相邻隐藏 tab，而不是仅移动文本。
4. 使用 `Ctrl+PageUp` / `Ctrl+PageDown` 连续切换并循环，确认活动 tab 始终可见。
5. 使用 `[t` / `]t` 和 `gt` / `gT` 重复检查。
6. 关闭当前 tab，确认新的当前 tab 可见。
7. 缩窄和放宽终端，确认箭头自动出现/消失，命中位置与视觉一致。
8. 点击可见 tab 的标题与关闭按钮，确认没有点位偏移。
9. 重启并恢复 workspace，确认恢复的当前 tab 首帧可见。

Expected: 无半个 tab（单项超宽的受控截断除外）、无零宽点击区、箭头不覆盖关闭按钮、切换无额外闪动。

**Step 6: 检查最终 diff**

Run: `git diff -- src/ui/mod.rs src/ui/icons.rs src/input/mouse.rs tests/ui_render.rs tests/mouse.rs docs/plans/2026-09-02-workspace-tab-overflow-implementation.md`

Expected: 仅包含 tab overflow、图标、鼠标映射、测试和本计划；不包含 workspace schema、`Action` 新变体或无关格式化。

## Acceptance Criteria

- 所有 tab 可容纳时，tab bar 与当前行为一致且不显示导航箭头。
- 溢出时固定显示左右箭头槽，enabled/disabled 状态准确。
- 任意合法 `active_tab` 在任意支持的终端宽度下都处于渲染窗口内。
- `NextTab`、`PreviousTab`、`ActivateTab` 后无需额外同步动作即可显示新的当前 tab。
- 点击箭头激活对应方向最近的隐藏 tab。
- 不可见 tab 和关闭按钮没有 `HitRegion`。
- 所有 tab bar `HitRegion` 宽度大于 0，且不越过 tab 内容区或箭头区域。
- 关闭、新建、workspace 恢复、循环切换和 resize 后当前 tab 都可见。
- ASCII、Unicode、Nerd Font 模式均正确计算 cell width。
- 不修改持久化版本，不新增 UI 滚动状态，不新增 tab 切换快捷键。
- `cargo fmt --all -- --check`、Clippy、定向测试和完整测试集全部通过。

## Suggested Commit Boundaries

仅在用户明确要求提交时创建 commit；建议拆分为：

1. `test(ui): cover workspace tab overflow viewport`
2. `feat(ui): keep active workspace tab visible`
3. `feat(mouse): navigate overflowing workspace tabs`
4. `test(ui): cover tab overflow state transitions`
