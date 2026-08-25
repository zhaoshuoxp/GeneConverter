#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText};
use gene_converter::{
    ConversionProgress, ConversionRequest, ConversionSummary, Direction, Preview, Species,
    convert_file, load_preview, suggested_output_path,
};
use rfd::FileDialog;

const ACCENT: Color32 = Color32::from_rgb(46, 125, 231);
const PREVIEW_ROWS: usize = 10;

fn main() -> eframe::Result {
    let initial_file = std::env::args_os().nth(1).map(PathBuf::from);
    eframe::run_native(
        "Gene ID / Symbol Converter",
        native_options(),
        Box::new(move |creation_context| {
            Ok(Box::new(GeneConverterApp::new(
                creation_context,
                initial_file,
            )))
        }),
    )
}

fn native_options() -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("gene-converter")
            .with_title("Gene ID / Symbol Converter")
            .with_inner_size([1_080.0, 780.0])
            .with_min_inner_size([760.0, 620.0]),
        renderer: eframe::Renderer::Glow,
        centered: true,
        // eframe's window-state persistence triggers an AppKit Touch Bar KVO
        // cleanup crash on some Intel Macs when the final window is closed.
        // This application has no persisted window state, so disable that path.
        persist_window: false,
        // A desktop executable never needs to return to its caller and open a
        // second event loop. Exiting the process directly also prevents AppKit
        // from running the faulty post-window Touch Bar observer flush.
        run_and_return: false,
        ..Default::default()
    }
}

#[derive(Clone)]
enum Notice {
    Error(String),
    Info(String),
}

enum WorkerEvent {
    Stage(&'static str),
    Progress(ConversionProgress),
    Done(Result<ConversionSummary, String>),
}

struct ActiveJob {
    receiver: mpsc::Receiver<WorkerEvent>,
    cancel: Arc<AtomicBool>,
    progress: ConversionProgress,
    stage: &'static str,
}

struct GeneConverterApp {
    input: Option<PathBuf>,
    output_directory: Option<PathBuf>,
    preview: Option<Preview>,
    selected_column: usize,
    species: Species,
    direction: Direction,
    keep_version: bool,
    notice: Option<Notice>,
    pending_overwrite: Option<ConversionRequest>,
    active_job: Option<ActiveJob>,
    last_summary: Option<ConversionSummary>,
}

impl GeneConverterApp {
    fn new(creation_context: &eframe::CreationContext<'_>, initial_file: Option<PathBuf>) -> Self {
        install_fallback_font(&creation_context.egui_ctx);
        creation_context.egui_ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(10.0, 10.0);
            style.spacing.button_padding = egui::vec2(14.0, 8.0);
            style.visuals.selection.bg_fill = ACCENT;
            style.visuals.hyperlink_color = ACCENT;
        });

        let mut app = Self {
            input: None,
            output_directory: None,
            preview: None,
            selected_column: 0,
            species: Species::Human,
            direction: Direction::IdToSymbol,
            keep_version: true,
            notice: None,
            pending_overwrite: None,
            active_job: None,
            last_summary: None,
        };
        if let Some(path) = initial_file.filter(|path| path.is_file()) {
            app.open_file(path);
        }
        app
    }

    fn choose_file(&mut self) {
        let mut dialog = FileDialog::new()
            .set_title("Select a CSV or TSV file")
            .add_filter("Delimited text", &["csv", "tsv", "txt"]);
        if let Some(parent) = self.input.as_deref().and_then(Path::parent) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(path) = dialog.pick_file() {
            self.open_file(path);
        }
    }

    fn choose_output_directory(&mut self) {
        let starting_directory = self
            .output_directory
            .as_deref()
            .or_else(|| self.input.as_deref().and_then(Path::parent));
        let mut dialog = FileDialog::new().set_title("Choose output folder");
        if let Some(directory) = starting_directory {
            dialog = dialog.set_directory(directory);
        }
        if let Some(directory) = dialog.pick_folder() {
            self.output_directory = Some(directory);
        }
    }

    fn open_file(&mut self, path: PathBuf) {
        match load_preview(&path, PREVIEW_ROWS) {
            Ok(preview) => {
                self.input = Some(path);
                self.preview = Some(preview);
                self.selected_column = 0;
                self.last_summary = None;
                self.notice = None;
            }
            Err(error) => {
                self.notice = Some(Notice::Error(format!(
                    "Could not load {}\n\n{error:#}",
                    path.display()
                )));
            }
        }
    }

    fn accept_dropped_files(&mut self, context: &egui::Context) {
        let dropped = context.input(|input| input.raw.dropped_files.clone());
        let Some(path) = dropped
            .into_iter()
            .map(|file| file.path().to_path_buf())
            .find(|path| !path.as_os_str().is_empty())
        else {
            return;
        };
        let supported = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                ["csv", "tsv", "txt"]
                    .iter()
                    .any(|expected| extension.eq_ignore_ascii_case(expected))
            });
        if supported {
            self.open_file(path);
        } else {
            self.notice = Some(Notice::Error(
                "Please drop a .csv, .tsv, or .txt file.".to_owned(),
            ));
        }
    }

    fn build_request(&self) -> Option<ConversionRequest> {
        let input = self.input.clone()?;
        let preview = self.preview.as_ref()?;
        Some(ConversionRequest {
            output: suggested_output_path(&input, self.output_directory.as_deref()),
            input,
            column_index: self.selected_column,
            species: self.species,
            direction: self.direction,
            keep_version: self.keep_version,
            delimiter: preview.delimiter,
        })
    }

    fn request_conversion(&mut self, context: &egui::Context) {
        let Some(request) = self.build_request() else {
            self.notice = Some(Notice::Error("Select an input file first.".to_owned()));
            return;
        };
        if request.output.exists() {
            self.pending_overwrite = Some(request);
        } else {
            self.start_conversion(context, request);
        }
    }

    fn start_conversion(&mut self, context: &egui::Context, request: ConversionRequest) {
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_context = context.clone();

        std::thread::spawn(move || {
            let _ = sender.send(WorkerEvent::Stage("Preparing gene mapping…"));
            worker_context.request_repaint();
            let result = convert_file(
                &request,
                |progress| {
                    let _ = sender.send(WorkerEvent::Progress(progress));
                    worker_context.request_repaint();
                },
                || worker_cancel.load(Ordering::Relaxed),
            )
            .map_err(|error| format!("{error:#}"));
            let _ = sender.send(WorkerEvent::Done(result));
            worker_context.request_repaint();
        });

        self.last_summary = None;
        self.active_job = Some(ActiveJob {
            receiver,
            cancel,
            progress: ConversionProgress::default(),
            stage: "Starting…",
        });
    }

    fn poll_worker(&mut self) {
        let Some(job) = &mut self.active_job else {
            return;
        };
        let mut completion = None;
        while let Ok(event) = job.receiver.try_recv() {
            match event {
                WorkerEvent::Stage(stage) => job.stage = stage,
                WorkerEvent::Progress(progress) => {
                    job.stage = "Converting rows…";
                    job.progress = progress;
                }
                WorkerEvent::Done(result) => completion = Some(result),
            }
        }

        if let Some(result) = completion {
            self.active_job = None;
            match result {
                Ok(summary) => self.last_summary = Some(summary),
                Err(message) if message == "conversion cancelled" => {
                    self.notice = Some(Notice::Info("Conversion cancelled.".to_owned()));
                }
                Err(message) => self.notice = Some(Notice::Error(message)),
            }
        }
    }

    fn show_header(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.heading(RichText::new("Gene ID / Symbol Converter").size(25.0));
                ui.label(
                    RichText::new("Fast local conversion · no data leaves your computer")
                        .weak()
                        .size(13.0),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("v2.0.1 · Rust").weak().monospace());
            });
        });
    }

    fn show_file_section(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Input file").strong());
                    if let Some(path) = &self.input {
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_default();
                        ui.label(RichText::new(name).size(15.0));
                        ui.label(RichText::new(path.display().to_string()).weak().size(11.0));
                    } else {
                        ui.label("Drop a CSV/TSV here, or choose a file");
                        ui.label(RichText::new("The first 10 rows will be previewed.").weak());
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            self.active_job.is_none(),
                            egui::Button::new("Choose file…").fill(ACCENT),
                        )
                        .clicked()
                    {
                        self.choose_file();
                    }
                });
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Output").strong());
                let label = self
                    .output_directory
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "Same folder as input".to_owned());
                ui.label(label);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.output_directory.is_some()
                        && ui
                            .add_enabled(self.active_job.is_none(), egui::Button::new("Reset"))
                            .clicked()
                    {
                        self.output_directory = None;
                    }
                    if ui
                        .add_enabled(
                            self.active_job.is_none(),
                            egui::Button::new("Choose folder…"),
                        )
                        .clicked()
                    {
                        self.choose_output_directory();
                    }
                });
            });
        });
    }

    fn show_options(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Conversion settings").strong());
            ui.add_space(2.0);
            egui::Grid::new("conversion_settings")
                .num_columns(4)
                .spacing([12.0, 10.0])
                .show(ui, |ui| {
                    ui.label("Genome build");
                    ui.add_enabled_ui(self.active_job.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("species")
                            .selected_text(self.species.label())
                            .width(225.0)
                            .show_ui(ui, |ui| {
                                for species in Species::ALL {
                                    ui.selectable_value(
                                        &mut self.species,
                                        species,
                                        species.label(),
                                    );
                                }
                            });
                    });
                    ui.label("Direction");
                    ui.add_enabled_ui(self.active_job.is_none(), |ui| {
                        egui::ComboBox::from_id_salt("direction")
                            .selected_text(self.direction.label())
                            .width(225.0)
                            .show_ui(ui, |ui| {
                                for direction in Direction::ALL {
                                    ui.selectable_value(
                                        &mut self.direction,
                                        direction,
                                        direction.label(),
                                    );
                                }
                            });
                    });
                    ui.end_row();

                    ui.label("Source column");
                    ui.add_enabled_ui(self.active_job.is_none(), |ui| {
                        let selected = self
                            .preview
                            .as_ref()
                            .and_then(|preview| preview.headers.get(self.selected_column))
                            .map(String::as_str)
                            .unwrap_or("Select a file first");
                        egui::ComboBox::from_id_salt("column")
                            .selected_text(selected)
                            .width(225.0)
                            .show_ui(ui, |ui| {
                                if let Some(preview) = &self.preview {
                                    for (index, header) in preview.headers.iter().enumerate() {
                                        ui.selectable_value(
                                            &mut self.selected_column,
                                            index,
                                            header,
                                        );
                                    }
                                }
                            });
                    });
                    ui.label("ID version");
                    ui.add_enabled_ui(
                        self.active_job.is_none() && self.direction == Direction::SymbolToId,
                        |ui| {
                            ui.checkbox(&mut self.keep_version, "Keep version suffix (.1, .2, …)");
                        },
                    );
                    ui.end_row();
                });
        });
    }

    fn show_preview(&self, ui: &mut egui::Ui) {
        let Some(preview) = &self.preview else {
            let panel_height = (ui.available_height() - 70.0).max(160.0);
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.set_min_height(panel_height);
                ui.vertical_centered(|ui| {
                    ui.add_space(((panel_height - 75.0) / 2.0).max(20.0));
                    ui.label(RichText::new("No preview yet").strong().size(16.0));
                    ui.label(RichText::new("Choose or drop a file to begin.").weak());
                    ui.add_space(30.0);
                });
            });
            return;
        };

        ui.horizontal(|ui| {
            ui.label(RichText::new("Data preview").strong());
            ui.label(
                RichText::new(format!(
                    "{} columns · first {} rows · {}",
                    preview.headers.len(),
                    preview.rows.len(),
                    format_bytes(preview.file_size)
                ))
                .weak(),
            );
        });
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt("preview_scroll")
                .auto_shrink([false, false])
                .max_height(280.0)
                .show(ui, |ui| {
                    egui::Grid::new("preview_grid")
                        .striped(true)
                        .spacing([10.0, 5.0])
                        .show(ui, |ui| {
                            ui.add_sized([34.0, 20.0], egui::Label::new("#"));
                            for header in &preview.headers {
                                ui.add_sized(
                                    [170.0, 20.0],
                                    egui::Label::new(RichText::new(header).strong()).truncate(),
                                )
                                .on_hover_text(header);
                            }
                            ui.end_row();

                            for (row_index, row) in preview.rows.iter().enumerate() {
                                ui.add_sized(
                                    [34.0, 20.0],
                                    egui::Label::new(
                                        RichText::new((row_index + 1).to_string()).weak(),
                                    ),
                                );
                                for column_index in 0..preview.headers.len() {
                                    let value = row.get(column_index).map_or("", String::as_str);
                                    ui.add_sized([170.0, 20.0], egui::Label::new(value).truncate())
                                        .on_hover_text(value);
                                }
                                ui.end_row();
                            }
                        });
                });
        });
    }

    fn show_action(&mut self, ui: &mut egui::Ui) {
        if let Some(job) = &self.active_job {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(job.stage).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            job.cancel.store(true, Ordering::Relaxed);
                        }
                        ui.label(format!("{} rows", job.progress.rows_processed));
                    });
                });
                let percent = job.progress.fraction();
                ui.add(
                    egui::ProgressBar::new(percent)
                        .animate(true)
                        .show_percentage(),
                );
            });
            return;
        }

        if let Some(summary) = self.last_summary.clone() {
            ui.group(|ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Conversion complete").strong().color(ACCENT));
                        ui.label(format!(
                            "{} rows · {} converted · {} unchanged",
                            summary.rows_processed,
                            summary.values_converted,
                            summary.values_unmatched
                        ));
                        ui.label(RichText::new(summary.output.display().to_string()).weak());
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Convert again").clicked() {
                            self.last_summary = None;
                        }
                    });
                });
            });
            return;
        }

        let output_name = self
            .build_request()
            .and_then(|request| request.output.file_name().map(|name| name.to_owned()))
            .map(|name| name.to_string_lossy().into_owned());
        ui.horizontal(|ui| {
            if let Some(output_name) = output_name {
                ui.label(RichText::new(format!("Output: {output_name}")).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = egui::Button::new(RichText::new("Convert file").strong())
                    .fill(ACCENT)
                    .min_size(egui::vec2(180.0, 38.0));
                if ui.add_enabled(self.preview.is_some(), button).clicked() {
                    self.request_conversion(ui.ctx());
                }
            });
        });
    }

    fn show_overwrite_confirmation(&mut self, context: &egui::Context) {
        let Some(request) = self.pending_overwrite.clone() else {
            return;
        };
        let mut overwrite = false;
        let mut cancel = false;
        egui::Window::new("Replace existing output?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.label("The output file already exists:");
                ui.label(RichText::new(request.output.display().to_string()).monospace());
                ui.label("Replacing it cannot be undone.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui
                        .add(egui::Button::new("Replace").fill(Color32::from_rgb(190, 60, 60)))
                        .clicked()
                    {
                        overwrite = true;
                    }
                });
            });
        if overwrite {
            self.pending_overwrite = None;
            self.start_conversion(context, request);
        } else if cancel {
            self.pending_overwrite = None;
        }
    }

    fn show_notice(&mut self, context: &egui::Context) {
        let Some(notice) = self.notice.clone() else {
            return;
        };
        let (title, message) = match notice {
            Notice::Error(message) => ("Something went wrong", message),
            Notice::Info(message) => ("GeneConverter", message),
        };
        let mut close = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                ui.set_max_width(520.0);
                ui.label(message);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
        if close {
            self.notice = None;
        }
    }
}

impl eframe::App for GeneConverterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker();
        self.accept_dropped_files(ui.ctx());

        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(4.0);
            self.show_header(ui);
            ui.add_space(8.0);
            self.show_file_section(ui);
            ui.add_space(4.0);
            self.show_options(ui);
            ui.add_space(4.0);
            self.show_preview(ui);
            ui.add_space(4.0);
            self.show_action(ui);
        });

        self.show_overwrite_confirmation(ui.ctx());
        self.show_notice(ui.ctx());
    }
}

fn install_fallback_font(context: &egui::Context) {
    #[cfg(target_os = "macos")]
    const CANDIDATES: &[&str] = &[
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
    ];
    #[cfg(target_os = "windows")]
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\seguiemj.ttf",
    ];
    #[cfg(target_os = "linux")]
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    const CANDIDATES: &[&str] = &[];

    let Some(bytes) = CANDIDATES.iter().find_map(|path| fs::read(path).ok()) else {
        return;
    };
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "system_fallback".to_owned(),
        FontData::from_owned(bytes).into(),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("system_fallback".to_owned());
    }
    context.set_fonts(fonts);
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_persistence_stays_disabled_for_safe_macos_shutdown() {
        let options = super::native_options();
        assert!(!options.persist_window);
        assert!(!options.run_and_return);
    }
}
