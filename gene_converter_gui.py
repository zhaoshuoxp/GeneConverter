#!/usr/bin/env python3
# -*- coding: utf-8 -*-

"""
Gene ID/Symbol Converter GUI (PyQt5 version)
--------------------------------------------
Features:
- Select species: hg38_v43 or mm10_v25
- Load one input file (CSV/TSV)
- Preview first 10 rows of the file
- User manually selects which column to convert
- Choose conversion direction: ID → Symbol or Symbol → ID
- Optional "Keep version number" when converting Symbol → ID
- Optional output folder selection (default: same as input file)
- Internal mapping tables included for packaging
- Deduplication ensures map operations do not raise errors
"""

import sys
import os
import re
import pandas as pd
from PyQt5.QtWidgets import (
    QApplication, QWidget, QPushButton, QFileDialog, QLabel,
    QVBoxLayout, QHBoxLayout, QTableWidget, QTableWidgetItem,
    QComboBox, QMessageBox, QCheckBox
)
from PyQt5.QtCore import Qt

# -----------------------------
# Internal resource path handling (for PyInstaller)
# -----------------------------
def get_mapping_path(species):
    if getattr(sys, 'frozen', False):
        base_path = sys._MEIPASS
    else:
        base_path = os.path.dirname(__file__)
    if species == 'hg38_v43':
        return os.path.join(base_path, 'hg38_table.tsv')
    else:
        return os.path.join(base_path, 'mm10_table.tsv')

# -----------------------------
# Main GUI Window
# -----------------------------
class GeneConverterApp(QWidget):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Gene ID/Symbol Converter")
        self.resize(800, 600)
        self.df = None
        self.input_file_path = None
        self.output_folder = None

        layout = QVBoxLayout()

        # File selection layout
        file_layout = QHBoxLayout()
        self.file_label = QLabel("No file selected")
        self.load_btn = QPushButton("Select File")
        self.load_btn.clicked.connect(self.load_file)
        file_layout.addWidget(self.file_label)
        file_layout.addWidget(self.load_btn)
        layout.addLayout(file_layout)

        # Output folder selection
        out_layout = QHBoxLayout()
        self.output_label = QLabel("Output folder: (default: input file folder)")
        self.out_btn = QPushButton("Choose Folder")
        self.out_btn.clicked.connect(self.choose_output_folder)
        out_layout.addWidget(self.output_label)
        out_layout.addWidget(self.out_btn)
        layout.addLayout(out_layout)

        # Species selection
        species_layout = QHBoxLayout()
        self.species_combo = QComboBox()
        self.species_combo.addItems(['hg38_v43', 'mm10_v25'])
        species_layout.addWidget(QLabel("Select Genome Build:"))
        species_layout.addWidget(self.species_combo)
        layout.addLayout(species_layout)

        # Table preview
        self.table_widget = QTableWidget()
        layout.addWidget(self.table_widget)

        # Column selection and conversion direction
        col_layout = QHBoxLayout()
        self.col_combo = QComboBox()
        self.direction_combo = QComboBox()
        self.direction_combo.addItems(['ID → Symbol', 'Symbol → ID'])
        col_layout.addWidget(QLabel("Select Column:"))
        col_layout.addWidget(self.col_combo)
        col_layout.addWidget(QLabel("Conversion Direction:"))
        col_layout.addWidget(self.direction_combo)
        layout.addLayout(col_layout)

        # Keep version option
        self.keep_version_check = QCheckBox("Keep version number (Symbol → ID only)")
        self.keep_version_check.setChecked(True)
        layout.addWidget(self.keep_version_check)

        # Convert button
        self.convert_btn = QPushButton("Convert")
        self.convert_btn.clicked.connect(self.convert_file)
        layout.addWidget(self.convert_btn)

        self.setLayout(layout)

    # -----------------------------
    # Load CSV/TSV file
    # -----------------------------
    def load_file(self):
        file_path, _ = QFileDialog.getOpenFileName(
            self, "Select CSV/TSV File", "", "CSV/TSV Files (*.csv *.tsv *.txt)"
        )
        if not file_path:
            return
        self.input_file_path = file_path
        self.file_label.setText(os.path.basename(file_path))

        # Auto-detect separator
        sep = ',' if file_path.lower().endswith('.csv') else '\t'
        self.df = pd.read_csv(file_path, sep=sep, dtype=str)

        # Display preview
        self.display_preview()

        # Update column selection dropdown
        self.col_combo.clear()
        self.col_combo.addItems(self.df.columns.tolist())

    # -----------------------------
    # Display first 10 rows preview
    # -----------------------------
    def display_preview(self):
        if self.df is None:
            return
        preview = self.df.head(10)
        self.table_widget.setRowCount(preview.shape[0])
        self.table_widget.setColumnCount(preview.shape[1])
        self.table_widget.setHorizontalHeaderLabels(preview.columns.tolist())
        for i in range(preview.shape[0]):
            for j in range(preview.shape[1]):
                item = QTableWidgetItem(str(preview.iat[i, j]))
                self.table_widget.setItem(i, j, item)
        self.table_widget.resizeColumnsToContents()

    # -----------------------------
    # Choose output folder
    # -----------------------------
    def choose_output_folder(self):
        folder = QFileDialog.getExistingDirectory(
            self, "Select Output Folder", ""
        )
        if folder:
            self.output_folder = folder
            self.output_label.setText(f"Output folder: {folder}")

    # -----------------------------
    # Perform conversion
    # -----------------------------
    def convert_file(self):
        if self.df is None or self.input_file_path is None:
            QMessageBox.warning(self, "Warning", "Please select a file first")
            return

        col_name = self.col_combo.currentText()
        direction = self.direction_combo.currentText()
        species = self.species_combo.currentText()
        mapping_path = get_mapping_path(species)

        if not os.path.exists(mapping_path):
            QMessageBox.warning(self, "Warning", f"Mapping file not found: {mapping_path}")
            return

        # Load mapping table
        mapping = pd.read_csv(mapping_path, sep='\t', header=None, names=['ensembl', 'symbol'], dtype=str)

        # Build lookup dicts
        mapping_noversion = mapping.copy()
        mapping_noversion['ensembl_base'] = mapping_noversion['ensembl'].str.replace(r"\.\d+$", "", regex=True)

        id2symbol = mapping_noversion.drop_duplicates(subset="ensembl_base").set_index("ensembl_base")['symbol'].to_dict()
        symbol2id = mapping.drop_duplicates(subset="symbol").set_index("symbol")['ensembl'].to_dict()

        # Copy dataframe to output
        df_out = self.df.copy()

        if direction == 'ID → Symbol':
            def id_to_symbol(val):
                if pd.isna(val):
                    return val
                val_base = re.sub(r"\.\d+$", "", str(val))
                return id2symbol.get(val, id2symbol.get(val_base, val))
            new_col = col_name + '_symbol'
            df_out[new_col] = df_out[col_name].map(id_to_symbol)
        else:  # Symbol → ID
            keep_version = self.keep_version_check.isChecked()
            def symbol_to_id(val):
                if pd.isna(val):
                    return val
                ensg = symbol2id.get(str(val), val)
                if not keep_version:
                    ensg = re.sub(r"\.\d+$", "", ensg)
                return ensg
            new_col = col_name + '_ensembl'
            df_out[new_col] = df_out[col_name].map(symbol_to_id)

        # Determine output folder
        output_folder = self.output_folder if self.output_folder else os.path.dirname(self.input_file_path)
        base_name = os.path.splitext(os.path.basename(self.input_file_path))[0]
        ext = os.path.splitext(self.input_file_path)[1]
        output_file = os.path.join(output_folder, base_name + '_converted' + ext)

        # Save file
        sep = ',' if ext.lower() == '.csv' else '\t'
        df_out.to_csv(output_file, sep=sep, index=False)

        QMessageBox.information(self, "Done", f"Conversion completed.\nOutput file: {output_file}")

# -----------------------------
# Program Entry
# -----------------------------
if __name__ == '__main__':
    app = QApplication(sys.argv)
    window = GeneConverterApp()
    window.show()
    sys.exit(app.exec_())
