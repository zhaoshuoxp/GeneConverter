# GeneConverter

一个快速、原生、跨平台的基因 ID / Symbol 转换桌面工具。2.0.1 版本已由 Python + PyQt5/pandas 重构为 Rust + egui，同一套代码可构建 Windows、macOS（Apple Silicon / Intel）和 Linux 应用。

## 功能

- Ensembl ID → Gene Symbol
- Gene Symbol / 别名 → Ensembl ID
- `hg38 / GENCODE v43` 与 `mm10 / GENCODE v25`
- CSV、TSV、TXT 文件，保留引号、逗号和空值等字段结构
- 前 10 行数据预览与待转换列选择
- 可选保留 Ensembl 版本号
- 多个匹配结果去重后以逗号连接
- 文件拖放、后台转换、实时进度、取消和覆盖确认
- 映射表内嵌在应用中；转换完全离线，不上传数据

## 为什么重构

旧版本每次转换都会由 pandas 完整加载输入和映射表，并依赖体积较大的 Python/PyQt 运行时。新版本：

- 流式读写输入文件，内存占用不再随输入文件大小线性增长；
- 映射表按所选物种懒加载，并在后续转换中复用缓存；
- 发布物是原生可执行程序，无需用户安装 Python；
- 核心转换逻辑与 GUI 分离，能独立测试，也便于未来增加命令行或新物种。

## 直接运行

需要 [Rust stable](https://www.rust-lang.org/tools/install)（最低 1.95）：

```bash
cargo run --release
```

首次构建会下载 Rust 依赖。映射文件 `hg38_table.csv` 和 `mm10_table.csv` 在编译时嵌入程序。

Linux 构建前需要窗口系统依赖。Ubuntu/Debian：

```bash
sudo apt-get update
sudo apt-get install -y libgtk-3-dev libxkbcommon-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libssl-dev
```

## 使用

1. 拖入或选择 `.csv`、`.tsv`、`.txt` 文件。
2. 选择基因组版本、转换方向和源列。
3. Symbol → ID 时选择是否保留 `.1`、`.2` 等版本后缀。
4. 需要时选择输出目录，然后点击 **Convert file**。

结果默认写入输入文件同目录，文件名为 `<原文件名>_converted.<扩展名>`。未匹配的值保持原样，新列名为 `<源列>_symbol` 或 `<源列>_ensembl`。

## 测试与构建

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

macOS `.app` 可使用 `cargo-bundle`：

```bash
cargo install cargo-bundle
cargo bundle --release
```

应用位于 `target/release/bundle/osx/GeneConverter.app`。Windows 可执行文件位于 `target/release/gene-converter.exe`，Linux 可执行文件位于 `target/release/gene-converter`。

仓库中的 GitHub Actions 会在四种环境中测试和打包：

- Windows x86_64
- macOS Apple Silicon
- macOS Intel
- Linux x86_64

推送形如 `v2.0.1` 的 tag 时会自动创建 GitHub Release 并附上四个平台的压缩包。

## 项目结构

```text
src/lib.rs                  流式转换核心、映射缓存与单元测试
src/main.rs                 跨平台原生 GUI
hg38_table.csv              人类基因映射表
mm10_table.csv              小鼠基因映射表
.github/workflows/build.yml 三平台持续集成与发布
```

## License

MIT
