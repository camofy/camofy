# Camofy Web 配色方案（基于 logo）

## 1. Logo 取色

- 绿叶主色：`#b7c1a9`
- 绿叶轮廓：`#818375`
- 花朵主色：`#f1ded0`
- 花朵轮廓：`#c1ab9d`
- 背景：`#ffffff`

## 2. 网站主题色板

- `--color-bg-app`：`#ffffff`  
  - 全局页面背景（`app-root`、主体背景）。
- `--color-surface`：`#f6f2ec`  
  - 卡片、面板背景（各个 section 容器、表单块）。
- `--color-surface-soft`：`#fbf7f1`  
  - 更浅的卡片背景、强调区域 hover 背景。
- `--color-border-subtle`：`#e0d4c6`  
  - 默认边框（面板边框、输入框边框）。
- `--color-border-strong`：`#c1ab9d`（花朵轮廓）  
  - 主要强调边框、选中状态的边框。

- `--color-text-main`：`#2b2a28`  
  - 主体文字（标题、正文）。
- `--color-text-muted`：`#7c7369`  
  - 次要文字、说明文字。
- `--color-text-soft`：`#ada397`  
  - 更弱的提示性文字。

- `--color-primary`：`#818375`（绿叶轮廓）  
  - 主要按钮背景、导航高亮、链接主色。
- `--color-primary-soft`：`#b7c1a9`（绿叶主色）  
  - 主按钮的浅色背景、标签背景。
- `--color-primary-on`：`#ffffff`  
  - 主按钮、主色背景上的文字颜色。

- `--color-accent`：`#f1ded0`（花朵主色）  
  - 轻量强调区背景、徽标式高亮背景。
- `--color-accent-strong`：`#c1ab9d`（花朵轮廓）  
  - 强调文字、提示标签框线。

- `--color-success`：`#6f8b63`  
  - 成功状态文字/图标（由绿叶主色加深）。
- `--color-success-soft`：`#d7e2cf`  
  - 成功状态背景（浅绿色块）。
- `--color-danger`：`#bb6e5e`  
  - 错误/警告按钮、错误文字。
- `--color-danger-soft`：`#f7e0da`  
  - 错误背景（轻微错误提示块）。

- `--color-log-bg`：`#111827`  
  - 日志区域深色背景（保留深色以提高对比度）。
- `--color-log-border`：`#1f2937`  
  - 日志区域边框。
- `--color-log-text-main`：`#e5e7eb`  
  - 日志主体文字。
- `--color-log-text-muted`：`#9ca3af`  
  - 日志说明文字。

## 3. 主要使用位置说明

- 页面背景：  
  - `app-root` 使用 `--color-bg-app`。  
  - 顶部/底部区域使用相同或略深的 surface 色。

- 卡片 & 面板：  
  - 各种 Section（订阅管理、用户配置、内核管理、日志等）容器背景使用 `--color-surface`。  
  - Section 边框使用 `--color-border-subtle`，需要强调时使用 `--color-border-strong`。

- 导航 & 交互元素：  
  - 顶部导航激活态背景：`--color-primary`，文字 `--color-primary-on`。  
  - 导航 hover 背景：`--color-primary-soft`。  
  - 主按钮（确认、保存、登录等）：背景 `--color-primary`，文字 `--color-primary-on`，hover 使用略深的 `#6b6f63`。  
  - 次级按钮：边框 `--color-border-strong`，文字 `--color-text-main`，hover 使用 `--color-surface-soft`。

- 文本层级：  
  - 标题文字：`--color-text-main`。  
  - 普通说明文字：`--color-text-muted`。  
  - 辅助说明/占位文字：`--color-text-soft`。

- 状态与提示：  
  - 成功状态徽标/标签背景：`--color-success-soft`，文字 `--color-success`。  
  - 错误状态按钮/文字：`--color-danger`，浅错误背景 `--color-danger-soft`。  
  - 轻微提示块（如操作提示）：背景使用 `--color-accent` 或 `--color-accent-strong` 边框。

- 日志区域（深色块）：  
  - 背景：`--color-log-bg`。  
  - 边框：`--color-log-border`。  
  - 文本：主体 `--color-log-text-main`，说明文字 `--color-log-text-muted`。  
  - ANSI 颜色（终端颜色）映射保留深色背景，前景色在 `App.css` 中基于深色背景做适当调整，使其在 `--color-log-bg` 上仍然有良好对比度。

