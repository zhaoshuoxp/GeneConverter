//! High-performance conversion core used by the desktop application.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use csv::{ReaderBuilder, StringRecord, WriterBuilder};

const HG38_MAPPING: &[u8] = include_bytes!("../hg38_table.csv");
const MM10_MAPPING: &[u8] = include_bytes!("../mm10_table.csv");

static HG38_CACHE: OnceLock<Result<Mapping, String>> = OnceLock::new();
static MM10_CACHE: OnceLock<Result<Mapping, String>> = OnceLock::new();

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Species {
    #[default]
    Human,
    Mouse,
}

impl Species {
    pub const ALL: [Self; 2] = [Self::Human, Self::Mouse];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Human => "Human · hg38 / GENCODE v43",
            Self::Mouse => "Mouse · mm10 / GENCODE v25",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Human => "hg38_v43",
            Self::Mouse => "mm10_v25",
        }
    }
}

impl fmt::Display for Species {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.short_label())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Direction {
    #[default]
    IdToSymbol,
    SymbolToId,
}

impl Direction {
    pub const ALL: [Self; 2] = [Self::IdToSymbol, Self::SymbolToId];

    pub const fn label(self) -> &'static str {
        match self {
            Self::IdToSymbol => "Ensembl ID → Gene symbol",
            Self::SymbolToId => "Gene symbol → Ensembl ID",
        }
    }

    pub const fn output_suffix(self) -> &'static str {
        match self {
            Self::IdToSymbol => "symbol",
            Self::SymbolToId => "ensembl",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub delimiter: u8,
    pub file_size: u64,
}

#[derive(Clone, Debug)]
pub struct ConversionRequest {
    pub input: PathBuf,
    pub output: PathBuf,
    pub column_index: usize,
    pub species: Species,
    pub direction: Direction,
    pub keep_version: bool,
    pub delimiter: u8,
}

#[derive(Clone, Debug, Default)]
pub struct ConversionProgress {
    pub rows_processed: u64,
    pub bytes_processed: u64,
    pub total_bytes: u64,
}

impl ConversionProgress {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            return 0.0;
        }
        (self.bytes_processed as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
    }
}

#[derive(Clone, Debug)]
pub struct ConversionSummary {
    pub output: PathBuf,
    pub rows_processed: u64,
    pub values_converted: u64,
    pub values_unmatched: u64,
}

#[derive(Debug, Default)]
struct Mapping {
    id_with_version_to_symbol: HashMap<String, Vec<String>>,
    id_without_version_to_symbol: HashMap<String, Vec<String>>,
    symbol_to_id: HashMap<String, Vec<String>>,
    alias_to_id: HashMap<String, Vec<String>>,
}

impl Mapping {
    fn from_reader(reader: impl Read) -> Result<Self> {
        let mut csv = ReaderBuilder::new()
            .has_headers(false)
            .flexible(false)
            .from_reader(reader);
        let mut mapping = Self::default();

        for (index, record) in csv.records().enumerate() {
            let record = record.with_context(|| format!("invalid mapping row {}", index + 1))?;
            if record.len() != 4 {
                bail!(
                    "mapping row {} has {} columns; expected 4",
                    index + 1,
                    record.len()
                );
            }

            let id_without_version = record.get(0).unwrap_or_default().trim();
            let id_with_version = record.get(1).unwrap_or_default().trim();
            let symbol = record.get(2).unwrap_or_default().trim();
            let aliases = record.get(3).unwrap_or_default();

            if id_without_version.is_empty() || id_with_version.is_empty() || symbol.is_empty() {
                continue;
            }

            push_unique(
                &mut mapping.id_without_version_to_symbol,
                id_without_version,
                symbol,
            );
            push_unique(
                &mut mapping.id_with_version_to_symbol,
                id_with_version,
                symbol,
            );
            push_unique(&mut mapping.symbol_to_id, symbol, id_with_version);

            // Preserve the legacy application's semantics: aliases separated by commas
            // are individually searchable, including aliases stored in a quoted CSV cell.
            for alias in aliases.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                push_unique(&mut mapping.alias_to_id, alias, id_with_version);
            }
        }

        Ok(mapping)
    }

    fn id_to_symbol(&self, value: &str) -> Option<String> {
        let values = if value.contains('.') {
            self.id_with_version_to_symbol.get(value).or_else(|| {
                value
                    .split_once('.')
                    .and_then(|(id, _)| self.id_without_version_to_symbol.get(id))
            })
        } else {
            self.id_without_version_to_symbol.get(value)
        }?;
        Some(values.join(","))
    }

    fn symbol_to_id(&self, value: &str, keep_version: bool) -> Option<String> {
        let ids = self
            .symbol_to_id
            .get(value)
            .or_else(|| self.alias_to_id.get(value))?;

        if keep_version {
            return Some(ids.join(","));
        }

        let mut unversioned = Vec::with_capacity(ids.len());
        for id in ids {
            let value = id.split_once('.').map_or(id.as_str(), |(id, _)| id);
            if !unversioned.contains(&value) {
                unversioned.push(value);
            }
        }
        Some(unversioned.join(","))
    }

    fn convert(&self, value: &str, direction: Direction, keep_version: bool) -> Option<String> {
        match direction {
            Direction::IdToSymbol => self.id_to_symbol(value),
            Direction::SymbolToId => self.symbol_to_id(value, keep_version),
        }
    }
}

fn push_unique(map: &mut HashMap<String, Vec<String>>, key: &str, value: &str) {
    let values = map.entry(key.to_owned()).or_default();
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_owned());
    }
}

fn mapping_for(species: Species) -> Result<&'static Mapping> {
    let cached = match species {
        Species::Human => HG38_CACHE.get_or_init(|| {
            Mapping::from_reader(HG38_MAPPING).map_err(|error| format!("{error:#}"))
        }),
        Species::Mouse => MM10_CACHE.get_or_init(|| {
            Mapping::from_reader(MM10_MAPPING).map_err(|error| format!("{error:#}"))
        }),
    };

    cached.as_ref().map_err(|message| anyhow!(message.clone()))
}

pub fn delimiter_for_path(path: &Path) -> u8 {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("csv"))
    {
        b','
    } else {
        b'\t'
    }
}

pub fn load_preview(path: &Path, max_rows: usize) -> Result<Preview> {
    let delimiter = delimiter_for_path(path);
    let file = File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let file_size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let mut reader = ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(BufReader::new(file));

    let headers = reader
        .headers()
        .with_context(|| format!("cannot read the header of {}", path.display()))?
        .iter()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();

    if headers.is_empty() {
        bail!("the selected file has no columns");
    }

    let mut rows = Vec::with_capacity(max_rows);
    for record in reader.records().take(max_rows) {
        rows.push(
            record
                .with_context(|| format!("cannot preview {}", path.display()))?
                .iter()
                .map(ToOwned::to_owned)
                .collect(),
        );
    }

    Ok(Preview {
        headers,
        rows,
        delimiter,
        file_size,
    })
}

pub fn suggested_output_path(input: &Path, output_directory: Option<&Path>) -> PathBuf {
    let directory = output_directory
        .map(Path::to_path_buf)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    let extension = input.extension().and_then(|value| value.to_str());
    let file_name = match extension {
        Some(extension) if !extension.is_empty() => format!("{stem}_converted.{extension}"),
        _ => format!("{stem}_converted.tsv"),
    };
    directory.join(file_name)
}

pub fn convert_file(
    request: &ConversionRequest,
    mut on_progress: impl FnMut(ConversionProgress),
    is_cancelled: impl Fn() -> bool,
) -> Result<ConversionSummary> {
    if request.input == request.output {
        bail!("input and output paths must be different");
    }

    let mapping = mapping_for(request.species)?;
    let input = File::open(&request.input)
        .with_context(|| format!("cannot open {}", request.input.display()))?;
    let total_bytes = input.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let output_directory = request
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(output_directory)
        .with_context(|| format!("cannot create output folder {}", output_directory.display()))?;
    let mut temporary_output =
        tempfile::NamedTempFile::new_in(output_directory).with_context(|| {
            format!(
                "cannot create a temporary file in {}",
                output_directory.display()
            )
        })?;

    let mut reader = ReaderBuilder::new()
        .delimiter(request.delimiter)
        .flexible(true)
        .from_reader(BufReader::new(input));
    let mut writer = WriterBuilder::new()
        .delimiter(request.delimiter)
        .from_writer(temporary_output.as_file_mut());

    let input_headers = reader
        .headers()
        .with_context(|| format!("cannot read the header of {}", request.input.display()))?
        .clone();
    if request.column_index >= input_headers.len() {
        bail!(
            "selected column {} does not exist in a {}-column file",
            request.column_index + 1,
            input_headers.len()
        );
    }

    let mut output_headers = input_headers.clone();
    let selected_name = input_headers
        .get(request.column_index)
        .unwrap_or("converted");
    output_headers.push_field(&format!(
        "{}_{}",
        selected_name,
        request.direction.output_suffix()
    ));
    writer.write_record(&output_headers)?;

    let mut row = StringRecord::new();
    let mut rows_processed = 0_u64;
    let mut values_converted = 0_u64;
    let mut values_unmatched = 0_u64;

    loop {
        if is_cancelled() {
            drop(writer);
            bail!("conversion cancelled");
        }
        if !reader
            .read_record(&mut row)
            .with_context(|| format!("cannot read row {}", rows_processed + 2))?
        {
            break;
        }

        let original = row.get(request.column_index).unwrap_or_default();
        let converted = mapping.convert(original, request.direction, request.keep_version);
        let output_value = if let Some(converted) = converted {
            values_converted += 1;
            converted
        } else {
            values_unmatched += 1;
            original.to_owned()
        };

        row.push_field(&output_value);
        writer
            .write_record(&row)
            .with_context(|| format!("cannot write row {}", rows_processed + 2))?;
        row.clear();
        rows_processed += 1;

        if rows_processed.is_multiple_of(2_000) {
            on_progress(ConversionProgress {
                rows_processed,
                bytes_processed: reader.position().byte(),
                total_bytes,
            });
        }
    }

    writer
        .flush()
        .context("cannot finish writing the output file")?;
    drop(writer);
    temporary_output
        .persist(&request.output)
        .map_err(|error| error.error)
        .with_context(|| format!("cannot save {}", request.output.display()))?;
    on_progress(ConversionProgress {
        rows_processed,
        bytes_processed: total_bytes,
        total_bytes,
    });

    Ok(ConversionSummary {
        output: request.output.clone(),
        rows_processed,
        values_converted,
        values_unmatched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn mapping_supports_versions_aliases_and_deduplication() {
        let data = concat!(
            "ENSG1,ENSG1.1,ABC,ALIAS\n",
            "ENSG1,ENSG1.1,ABC,SECOND\n",
            "ENSG1,ENSG1.2,ABC,ALIAS\n",
        );
        let mapping = Mapping::from_reader(data.as_bytes()).unwrap();

        assert_eq!(mapping.id_to_symbol("ENSG1.1").as_deref(), Some("ABC"));
        assert_eq!(mapping.id_to_symbol("ENSG1.99").as_deref(), Some("ABC"));
        assert_eq!(
            mapping.symbol_to_id("ALIAS", true).as_deref(),
            Some("ENSG1.1,ENSG1.2")
        );
        assert_eq!(mapping.symbol_to_id("ABC", false).as_deref(), Some("ENSG1"));
    }

    #[test]
    fn embedded_human_mapping_has_expected_entry() {
        let mapping = mapping_for(Species::Human).unwrap();
        assert_eq!(
            mapping.id_to_symbol("ENSG00000291190.1").as_deref(),
            Some("A2MP1")
        );
        assert_eq!(
            mapping.symbol_to_id("A2MP", true).as_deref(),
            Some("ENSG00000291190.1,ENSG00000256069.8")
        );

        let mouse = mapping_for(Species::Mouse).unwrap();
        assert_eq!(
            mouse.id_to_symbol("ENSMUSG00000064336.1").as_deref(),
            Some("mt-Tf")
        );
    }

    #[test]
    fn preview_and_conversion_preserve_csv_fields() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("genes.csv");
        let output = directory.path().join("genes_converted.csv");
        fs::write(
            &input,
            "gene,note\nENSG00000291190.1,plain\nUNKNOWN,\"comma, value\"\n",
        )
        .unwrap();

        let preview = load_preview(&input, 10).unwrap();
        assert_eq!(preview.headers, ["gene", "note"]);
        assert_eq!(preview.rows[1][1], "comma, value");

        let summary = convert_file(
            &ConversionRequest {
                input,
                output: output.clone(),
                column_index: 0,
                species: Species::Human,
                direction: Direction::IdToSymbol,
                keep_version: true,
                delimiter: b',',
            },
            |_| {},
            || false,
        )
        .unwrap();

        assert_eq!(summary.rows_processed, 2);
        assert_eq!(summary.values_converted, 1);
        assert_eq!(summary.values_unmatched, 1);
        let result = fs::read_to_string(output).unwrap();
        assert_eq!(
            result,
            "gene,note,gene_symbol\nENSG00000291190.1,plain,A2MP1\nUNKNOWN,\"comma, value\",UNKNOWN\n"
        );
    }

    #[test]
    fn chooses_delimiter_and_output_name() {
        assert_eq!(delimiter_for_path(Path::new("A.CSV")), b',');
        assert_eq!(delimiter_for_path(Path::new("a.tsv")), b'\t');
        assert_eq!(
            suggested_output_path(Path::new("/tmp/a.tsv"), None),
            PathBuf::from("/tmp/a_converted.tsv")
        );
    }

    #[test]
    fn cancellation_preserves_an_existing_output() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("genes.csv");
        let output = directory.path().join("genes_converted.csv");
        fs::write(&input, "gene\nENSG00000291190.1\n").unwrap();
        fs::write(&output, "do not replace").unwrap();

        let error = convert_file(
            &ConversionRequest {
                input,
                output: output.clone(),
                column_index: 0,
                species: Species::Human,
                direction: Direction::IdToSymbol,
                keep_version: true,
                delimiter: b',',
            },
            |_| {},
            || true,
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "conversion cancelled");
        assert_eq!(fs::read_to_string(output).unwrap(), "do not replace");
    }
}
