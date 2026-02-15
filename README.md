# Auto Wallpaper 

自动从 Bing 获取每日壁纸并设置为桌面壁纸的 Windows 工具。

## 功能

- **每日壁纸下载** — 从 Bing HPImageArchive API 获取 UHD 质量壁纸
- **自动设置壁纸** — 通过 Windows API 设置桌面壁纸并验证
- **水印支持** — 图片水印和文字水印（支持 bold/thin/normal 字重）
- **状态追踪** — 避免重复下载，支持断点恢复
- **文件归档** — 自动归档过期的壁纸文件夹
- **配置热修复** — 自动修复损坏或不完整的配置文件
- **后置执行** — 壁纸更换后可运行自定义程序
- **多路径复制** — 将壁纸复制到桌面及自定义路径

## 项目结构

```
rs/
├── .cargo/config.toml    # 构建配置（静态CRT链接）
├── Cargo.toml             # 依赖与 release 优化配置
├── src/
│   ├── main.rs            # 入口、流程编排、状态管理
│   ├── config.rs          # 配置加载、验证、自动修复
│   ├── logger.rs          # 带时间戳的文件日志
│   ├── download.rs        # HTTP 下载（带重试）
│   ├── wallpaper.rs       # Windows 壁纸 API（FFI）
│   ├── watermark.rs       # 图片/文字水印渲染
│   └── archive.rs         # 旧文件夹归档
└── README.md
```

## 依赖

| 依赖 | 用途 |
|------|------|
| `ureq` | 轻量 HTTP 客户端 |
| `serde` + `serde_json` | JSON 序列化/反序列化 |
| `image` | JPEG/PNG 图片处理 |
| `ab_glyph` | 字体加载与文字渲染 |
| `chrono` | 日期时间处理 |

> Windows API (`SystemParametersInfoW`、注册表访问) 通过手动 FFI 声明实现，无需 `windows-sys` 依赖。

## 构建

```bash
# Debug 构建
cargo build

# Release 构建（启用 LTO、符号裁剪、size 优化）
cargo build --release
```

Release 配置：
- `opt-level = "z"` — 优化体积
- `lto = true` — 链接时优化
- `codegen-units = 1` — 单编译单元
- `panic = "abort"` — 移除 panic 展开代码
- `strip = true` — 裁剪所有调试符号
- 静态链接 CRT（通过 `.cargo/config.toml`）

## 配置文件

程序运行时在 exe 所在目录查找 `config.json`，不存在则自动创建默认配置：

```json
{
    "idx": 0,
    "mkt": "zh-CN",
    "chk": true,
    "ctd": true,
    "wtm": false,
    "retry_delay": 3,
    "retry_count": 10,
    "watermarks": [
        {
            "type": "image",
            "path": "watermark1.png",
            "posX": 2.0,
            "posY": 1.2,
            "opacity": 50
        },
        {
            "type": "text",
            "content": "Sample Text Watermark",
            "posX": 2.0,
            "posY": 1.5,
            "opacity": 75,
            "font_type": "arial.ttf",
            "font_size": 46,
            "font_color": [128, 128, 128, 192],
            "font_weight": "normal"
        }
    ],
    "post_execution_apps": [],
    "copy_to_paths": []
}
```

### 配置项说明

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `idx` | 0-7 | 0 | Bing 壁纸索引（0=今天, 1=昨天...） |
| `mkt` | string | `"zh-CN"` | 市场代码 |
| `chk` | bool | `true` | 是否检查今日壁纸已完成 |
| `ctd` | bool | `true` | 是否复制壁纸到桌面 |
| `wtm` | bool | `false` | 是否添加水印 |
| `retry_delay` | int | 3 | 下载重试间隔（秒） |
| `retry_count` | int | 10 | 下载重试次数 |
| `watermarks` | array | — | 水印配置列表 |
| `post_execution_apps` | array | `[]` | 完成后运行的程序路径 |
| `copy_to_paths` | array | `[]` | 壁纸复制目标路径 |

### 水印类型

**图片水印** (`type: "image"`):
- `path` — 水印图片路径（相对于 exe 目录或绝对路径）
- `posX/posY` — 位置除数（`>0`，图片宽高除以此值得到坐标）
- `opacity` — 不透明度 `0-100`

**文字水印** (`type: "text"`):
- `content` — 水印文字
- `font_type` — 字体文件名（搜索 exe 目录和 Windows Fonts）
- `font_size` — 字号
- `font_color` — RGBA 颜色 `[R, G, B, A]`，0-255
- `font_weight` — `"normal"` | `"bold"` | `"thin"` | `"light"`

## 运行时文件结构

```
%APPDATA%/AutoWallpaper/
├── 2026.02.15/
│   ├── 2026.02.15.jpg        # 壁纸图片
│   ├── 2026.02.15_original.jpg  # 原始图片（开启水印时）
│   ├── 2026.02.15.log        # 运行日志
│   ├── api.json               # Bing API 响应
│   └── status.json            # 状态追踪
├── Archive/                   # 归档（超过10天的旧文件夹）
│   └── 2026/
│       └── 2026.02.05/
└── ...
```
