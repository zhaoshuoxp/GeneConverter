# GeneConverter

**Gene ID/Symbol Converter GUI for macOS**

A user-friendly GUI tool to convert gene identifiers between Ensembl IDs and gene symbols. Supports `hg38` and `mm10`.

- Load a CSV/TSV file
- Preview first 10 rows
- Select which column to convert
- Choose conversion direction: ID → Symbol or Symbol → ID
- Optional output folder (default: same directory as input)
- Includes internal mapping tables (`hg38_table.tsv`, `mm10_table.tsv`)

------

## Features

- Simple GUI interface using PySide6
- Deduplication to avoid mapping errors
- Internal mapping tables included for packaging
- Compatible with both Apple Silicon and Intel Macs (if packed properly)

------

## Requirements

- macOS 10.15+
- Python 3.10+ (Universal2 recommended)
- Dependencies: `pandas`, `PySide6`, `pyinstaller`

------

## Installation

1. Clone the repository:

```
git clone https://github.com/yourusername/GeneConverter.git
cd GeneConverter
```

1. Create a Python virtual environment:

```
python3 -m venv venv
source venv/bin/activate
pip install --upgrade pip
pip install pandas PySide6 pyinstaller
```

------

## Usage

Run the GUI:

```
python gene_converter_gui.py
```

- Click **Select File** to load CSV/TSV
- Choose species (`hg38` / `mm10`)
- Preview data and select column
- Choose conversion direction
- Optionally select output folder
- Click **Convert** to generate a new file with converted column

------

## Packaging as macOS `.app`

> Make sure to use **Universal2 Python** or Intel Python for cross-Mac compatibility.

### 1. Using PyInstaller:

```
pyinstaller --windowed --onefile --name GeneConverter \
--add-data "hg38_table.tsv:." \
--add-data "mm10_table.tsv:." \
gene_converter_gui.py
```

- `--windowed` → no terminal window
- `--onefile` → single executable
- `--add-data` → include mapping tables

The `.app` will appear in `dist/GeneConverter.app`.

------

### 2. Optional: Verify architecture

```
file dist/GeneConverter.app/Contents/MacOS/GeneConverter
```

- Apple Silicon only: `Mach-O 64-bit executable arm64`
- Intel only: `Mach-O 64-bit executable x86_64`
- Universal2: `Mach-O universal binary with 2 architectures: x86_64, arm64`

------

### 3. Optional: Create Universal2 `.app`

1. Build arm64 `.app` on Apple Silicon
2. Build x86_64 `.app` on Intel Mac
3. Merge using `lipo`:

```
lipo -create -output GeneConverter_universal \
arm64_bin x86_bin
cp GeneConverter_universal dist/GeneConverter.app/Contents/MacOS/GeneConverter
```

- Now the `.app` runs on both Intel and M1/M2 Macs

------

## License

MIT License