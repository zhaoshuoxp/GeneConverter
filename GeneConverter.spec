# -*- mode: python ; coding: utf-8 -*-


a = Analysis(
    ['gene_converter_gui.py'],
    pathex=[],
    binaries=[],
    datas=[('hg38_table.csv', '.'), ('mm10_table.csv', '.')],
    hiddenimports=[],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    [],
    exclude_binaries=True,
    name='GeneConverter',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    console=False,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    icon=['app.icns'],
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=True,
    upx_exclude=[],
    name='GeneConverter',
)
app = BUNDLE(
    coll,
    name='GeneConverter.app',
    icon='app.icns',
    bundle_identifier=None,
)
