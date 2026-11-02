// SPDX-License-Identifier: MIT OR Apache-2.0

//! CSV import and export (Phase 8): option panels, file pickers and
//! drag-and-drop of `.csv` / `.decel` files onto the window.

use std::path::{Path, PathBuf};

use decel_engine::protocol::{Command, CsvTarget, Reply};
use decel_io::{Encoding, ExportOptions, ImportOptions, Quote};
use decel_model::Rect;
use gpui::{
    ClickEvent, Context, ExternalPaths, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, px,
};
use gpui_component::ActiveTheme as _;

use crate::workspace::{Banner, Workspace};

pub struct ImportDialog {
    pub path: PathBuf,
    pub header: bool,
    pub new_sheet: bool,
    pub delimiter: Option<u8>,
    pub encoding: Encoding,
}

pub struct ExportDialog {
    pub path: PathBuf,
    pub raw: bool,
    pub selection_only: bool,
    pub delimiter: u8,
    pub quote: Quote,
}

const DELIMS: &[(Option<u8>, &str)] = &[
    (None, "Auto"),
    (Some(b','), ","),
    (Some(b';'), ";"),
    (Some(b'\t'), "Tab"),
    (Some(b'|'), "|"),
];

fn home() -> PathBuf {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
}

impl Workspace {
    pub(crate) fn prompt_import_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import CSV".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(p) = paths.into_iter().next() {
                        this.open_import_dialog(p, cx);
                    }
                }
                Ok(Err(e)) => {
                    this.banner = Some(Banner::Error(format!("file picker failed: {e}")));
                    cx.notify();
                }
                _ => {}
            });
        })
        .detach();
    }

    pub(crate) fn open_import_dialog(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.csv_import = Some(ImportDialog {
            path,
            header: false,
            new_sheet: true,
            delimiter: None,
            encoding: Encoding::Auto,
        });
        cx.notify();
    }

    pub(crate) fn prompt_export_csv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self
            .sheet_info_name()
            .map(|n| format!("{n}.csv"))
            .unwrap_or_else(|| "sheet.csv".into());
        let rx = cx.prompt_for_new_path(&home(), Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(Ok(Some(mut path))) => {
                    if path.extension().is_none() {
                        path.set_extension("csv");
                    }
                    this.csv_export = Some(ExportDialog {
                        path,
                        raw: true,
                        selection_only: false,
                        delimiter: b',',
                        quote: Quote::Necessary,
                    });
                    cx.notify();
                }
                Ok(Err(e)) => {
                    this.banner = Some(Banner::Error(format!("file picker failed: {e}")));
                    cx.notify();
                }
                _ => {}
            });
        })
        .detach();
    }

    fn run_import(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.csv_import.take() else {
            return;
        };
        let target = if d.new_sheet {
            CsvTarget::NewSheet
        } else {
            match self.addr(self.sel.rect().r1, self.sel.rect().c1) {
                Some(a) => CsvTarget::At(a),
                None => return,
            }
        };
        let options = ImportOptions {
            delimiter: d.delimiter,
            header: d.header,
            encoding: d.encoding,
        };
        self.request(
            Command::ImportCsv {
                path: d.path,
                target,
                options,
            },
            |this, r, window, cx| {
                match r {
                    Ok(Reply::Imported {
                        sheet,
                        rows,
                        rejected,
                    }) => {
                        this.status.last = if rejected > 0 {
                            format!("Imported {rows} rows ({rejected} cells rejected)")
                        } else {
                            format!("Imported {rows} rows")
                        };
                        if let Some(s) = sheet {
                            cx.defer_in(window, move |this, window, cx| {
                                this.switch_sheet(s, window, cx)
                            });
                        }
                    }
                    Err(e) => this.banner = Some(Banner::Error(format!("import failed: {e}"))),
                    _ => {}
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    fn run_export(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.csv_export.take() else {
            return;
        };
        let Some(sheet) = self.sheet else { return };
        let rect: Option<Rect> = d.selection_only.then(|| self.sel.rect());
        let path = d.path.clone();
        self.request(
            Command::ExportCsv {
                path: d.path,
                sheet,
                rect,
                raw: d.raw,
                options: ExportOptions {
                    delimiter: d.delimiter,
                    quote: d.quote,
                    ..ExportOptions::default()
                },
            },
            move |this, r, _, cx| {
                match r {
                    Ok(_) => this.status.last = format!("Exported {}", path.display()),
                    Err(e) => this.banner = Some(Banner::Error(format!("export failed: {e}"))),
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Files dropped on the window: `.decel` opens, `.csv`/`.tsv`/`.txt` imports.
    pub(crate) fn on_drop_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(p) = paths.paths().first().cloned() else {
            return;
        };
        match p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("decel") => self.open_path(p, false, window, cx),
            _ => self.open_import_dialog(p, cx),
        }
    }

    fn sheet_info_name(&self) -> Option<String> {
        let id = self.sheet?;
        Some(
            self.info
                .as_ref()?
                .sheets
                .iter()
                .find(|s| s.id == id)?
                .name
                .clone(),
        )
    }

    pub(crate) fn render_csv_dialog(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.csv_import.is_none() && self.csv_export.is_none() {
            return None;
        }
        let theme = cx.theme().clone();
        let chip = |id: String, label: &str, on: bool| {
            div()
                .id(SharedString::from(id))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .bg(if on { theme.primary } else { theme.secondary })
                .text_color(if on {
                    theme.primary_foreground
                } else {
                    theme.foreground
                })
                .child(label.to_string())
        };
        let row = |label: &str| {
            div().flex().flex_row().items_center().gap_1().child(
                div()
                    .w(px(90.))
                    .text_color(theme.muted_foreground)
                    .child(label.to_string()),
            )
        };
        let mut panel = div()
            .id("csv-dialog")
            .occlude()
            .absolute()
            .top(px(90.))
            .left(px(120.))
            .w(px(480.))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_md()
            .shadow_lg()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border);
        if let Some(d) = &self.csv_import {
            panel = panel
                .child(div().child(format!("Import {}", file_name(&d.path))))
                .child(
                    row("Into")
                        .child(chip("imp-new".into(), "New sheet", d.new_sheet).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                if let Some(d) = this.csv_import.as_mut() {
                                    d.new_sheet = true;
                                }
                                cx.notify();
                            }),
                        ))
                        .child(
                            chip("imp-at".into(), "At selection", !d.new_sheet).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(d) = this.csv_import.as_mut() {
                                        d.new_sheet = false;
                                    }
                                    cx.notify();
                                }),
                            ),
                        ),
                )
                .child(
                    row("Header row").child(
                        chip(
                            "imp-header".into(),
                            if d.header { "Yes" } else { "No" },
                            d.header,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(d) = this.csv_import.as_mut() {
                                    d.header = !d.header;
                                }
                                cx.notify();
                            },
                        )),
                    ),
                );
            let mut delims = row("Delimiter");
            for (i, (val, label)) in DELIMS.iter().enumerate() {
                let val = *val;
                delims = delims.child(
                    chip(format!("imp-d{i}"), label, d.delimiter == val).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Some(d) = this.csv_import.as_mut() {
                                d.delimiter = val;
                            }
                            cx.notify();
                        },
                    )),
                );
            }
            let mut enc = row("Encoding");
            for (i, (val, label)) in [
                (Encoding::Auto, "Auto"),
                (Encoding::Utf8, "UTF-8"),
                (Encoding::Windows1252, "Windows-1252"),
            ]
            .into_iter()
            .enumerate()
            {
                enc = enc.child(
                    chip(format!("imp-e{i}"), label, d.encoding == val).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Some(d) = this.csv_import.as_mut() {
                                d.encoding = val;
                            }
                            cx.notify();
                        },
                    )),
                );
            }
            panel =
                panel.child(delims).child(enc).child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(chip("imp-go".into(), "Import", true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.run_import(cx)),
                        ))
                        .child(
                            chip("imp-cancel".into(), "Cancel", false).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.csv_import = None;
                                    cx.notify();
                                },
                            )),
                        ),
                );
        } else if let Some(d) = &self.csv_export {
            panel =
                panel
                    .child(div().child(format!("Export to {}", file_name(&d.path))))
                    .child(
                        row("Write")
                            .child(chip("exp-raw".into(), "Raw values", d.raw).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(d) = this.csv_export.as_mut() {
                                        d.raw = true;
                                    }
                                    cx.notify();
                                }),
                            ))
                            .child(chip("exp-disp".into(), "Displayed text", !d.raw).on_click(
                                cx.listener(|this, _: &ClickEvent, _, cx| {
                                    if let Some(d) = this.csv_export.as_mut() {
                                        d.raw = false;
                                    }
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        row("Range").child(
                            chip(
                                "exp-sel".into(),
                                if d.selection_only {
                                    "Selection"
                                } else {
                                    "Whole sheet"
                                },
                                d.selection_only,
                            )
                            .on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    if let Some(d) = this.csv_export.as_mut() {
                                        d.selection_only = !d.selection_only;
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                    );
            let mut delims = row("Delimiter");
            for (i, (val, label)) in DELIMS.iter().skip(1).enumerate() {
                let val = val.unwrap_or(b',');
                delims = delims.child(
                    chip(format!("exp-d{i}"), label, d.delimiter == val).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Some(d) = this.csv_export.as_mut() {
                                d.delimiter = val;
                            }
                            cx.notify();
                        },
                    )),
                );
            }
            let mut quote = row("Quoting");
            for (i, (val, label)) in [
                (Quote::Necessary, "As needed"),
                (Quote::Always, "Always"),
                (Quote::NonNumeric, "Non-numeric"),
            ]
            .into_iter()
            .enumerate()
            {
                quote = quote.child(chip(format!("exp-q{i}"), label, d.quote == val).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(d) = this.csv_export.as_mut() {
                            d.quote = val;
                        }
                        cx.notify();
                    }),
                ));
            }
            panel =
                panel.child(delims).child(quote).child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(chip("exp-go".into(), "Export", true).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| this.run_export(cx)),
                        ))
                        .child(
                            chip("exp-cancel".into(), "Cancel", false).on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.csv_export = None;
                                    cx.notify();
                                },
                            )),
                        ),
                );
        }
        Some(panel)
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}
