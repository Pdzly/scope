//! Lazy inspection wiring and the Snap removal breakdown shared with previews.

use futures::{channel::mpsc, StreamExt};
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement, Context, FontWeight};

use crate::backend;
use crate::domain::package::PackageSource;
use crate::domain::snap_details::{DetailItem, RemovalEffect, SnapDetails};
use crate::theme;

use super::app_view::ScopeApp;

impl ScopeApp {
    pub(super) fn load_snap_details(&mut self, cx: &mut Context<Self>) {
        let pkg = self
            .selected_key
            .as_ref()
            .and_then(|key| {
                self.scan
                    .as_ref()?
                    .packages
                    .iter()
                    .find(|pkg| &pkg.key == key)
            })
            .cloned();
        let Some(pkg) = pkg.filter(|pkg| pkg.source == PackageSource::Snap) else {
            self.snap_details.cancel();
            return;
        };
        let Some(request) = self.snap_details.begin(&pkg) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            if request.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let (tx, mut rx) = mpsc::unbounded();
            backend::spawn_snap_details(pkg.package_id.clone(), request.cancel, tx);
            while let Some(update) = rx.next().await {
                this.update(cx, |this, cx| {
                    if this.snap_details.accept(
                        request.generation,
                        &pkg,
                        update.details,
                        update.complete,
                    ) {
                        if let Some(index) = this
                            .packages
                            .entries
                            .iter()
                            .position(|entry| entry.key == pkg.key)
                        {
                            this.list_state.splice(index..index + 1, 1);
                        }
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }
}

/// Compact type/path/size rows. Markers describe effects and are not controls.
pub(super) fn breakdown(details: Option<&SnapDetails>) -> AnyElement {
    let mut content = div()
        .flex()
        .flex_col()
        .gap(px(16.))
        .text_color(theme::text());
    if let Some(details) = details {
        for (effect, heading) in [
            (RemovalEffect::RemovedBySnap, ""),
            (RemovalEffect::Kept, "Kept after uninstall"),
            (RemovalEffect::Unverified, "Needs review"),
        ] {
            if details.items.iter().any(|item| item.effect == effect) {
                content = content.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .when(!heading.is_empty(), |this| {
                            this.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .pb(px(6.))
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(heading),
                            )
                        })
                        .children(
                            details
                                .items
                                .iter()
                                .filter(|item| item.effect == effect)
                                .map(detail_row),
                        ),
                );
            }
        }
        for warning in &details.warnings {
            if warning == crate::domain::snap_details::NO_SNAPSHOTS_NOTE
                || warning == crate::domain::snap_details::CLASSIC_COVERAGE_NOTE
            {
                continue;
            }
            content = content.child(
                div()
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme::text_dim())
                    .child(warning.clone()),
            );
        }
    } else {
        content = content.child(loading_rows());
    }
    content.into_any_element()
}

fn loading_rows() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .children(
            ["Application", "Older revisions", "User data", "System data"]
                .into_iter()
                .map(|label| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme::DETAIL_ROW_GAP))
                        .py(px(theme::DETAIL_ROW_PADDING))
                        .text_size(px(13.))
                        .line_height(px(18.))
                        .child(
                            div()
                                .flex_none()
                                .w(px(theme::DETAIL_LABEL_WIDTH))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(label),
                        )
                        .child(
                            div().flex_1().min_w(px(0.)).child(
                                div()
                                    .h(px(8.))
                                    .w_full()
                                    .max_w(px(300.))
                                    .rounded(px(4.))
                                    .bg(theme::border()),
                            ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .w(px(80.))
                                .text_right()
                                .text_size(px(12.))
                                .text_color(theme::text_dim())
                                .child("…"),
                        )
                }),
        )
        .into_any_element()
}

/// Compact estimate of managed items, not net disk space freed.
pub(super) fn removal_summary(details: Option<&SnapDetails>) -> String {
    let Some(details) = details else {
        return "Measuring removal…".into();
    };
    let size = details.removal_size();
    match size.bytes {
        Some(bytes) => format!(
            "Removes ~{}{}",
            measured_size(bytes),
            if size.complete { "" } else { "+" }
        ),
        None => "Removal size unknown".into(),
    }
}

fn detail_row(item: &DetailItem) -> AnyElement {
    let pending = item.size.note.as_deref() == Some(crate::domain::snap_details::MEASURING_NOTE);
    let size = match item.size.bytes {
        Some(bytes) if item.size.complete => measured_size(bytes),
        Some(bytes) => format!("{}+", measured_size(bytes)),
        None if pending => "Measuring…".into(),
        None => "Unknown".into(),
    };
    let icon = match item.effect {
        RemovalEffect::RemovedBySnap => None,
        RemovalEffect::Kept => Some(IconName::Minus),
        RemovalEffect::Unverified => Some(IconName::CircleAlert),
    };
    let location = item.location.as_deref().unwrap_or(&item.label);
    let location = if item.label.starts_with("Recovery snapshot") {
        format!("{location} · {}", item.label)
    } else {
        location.to_owned()
    };
    let mut path = div().flex_1().min_w(px(0.)).flex().flex_col().child(
        div()
            .text_size(px(12.))
            .line_height(px(18.))
            .text_color(theme::text_dim())
            .child(location),
    );
    if !item.size.complete && !pending {
        if let Some(note) = &item.size.note {
            path = path.child(
                div()
                    .text_size(px(11.))
                    .line_height(px(16.))
                    .text_color(theme::text_dim())
                    .child(note.clone()),
            );
        }
    } else if item.effect == RemovalEffect::Unverified {
        path = path.child(
            div()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(theme::text_dim())
                .child("Removal coverage is not verified"),
        );
    }
    div()
        .flex()
        .items_start()
        .gap(px(theme::DETAIL_ROW_GAP))
        .py(px(theme::DETAIL_ROW_PADDING))
        .text_size(px(13.))
        .line_height(px(18.))
        .child(
            div()
                .flex_none()
                .w(px(theme::DETAIL_LABEL_WIDTH))
                .flex()
                .items_center()
                .gap(px(9.))
                .children(icon.map(|icon| {
                    Icon::new(icon)
                        .size(px(12.))
                        .text_color(theme::text_faint())
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(item_kind(item)),
                ),
        )
        .child(path)
        .child(
            div()
                .flex_none()
                .w(px(80.))
                .text_right()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::text_dim())
                .child(size),
        )
        .into_any_element()
}

fn item_kind(item: &DetailItem) -> &'static str {
    if item.label.starts_with("Active revision") {
        "Application"
    } else if item.label.starts_with("Older revision") {
        "Older revision"
    } else if item.label.starts_with("Component") {
        "Component"
    } else if item.label.starts_with("Recovery snapshot") {
        "Recovery snapshot"
    } else if item.label.starts_with("System") {
        "System data"
    } else {
        "User data"
    }
}

fn measured_size(bytes: u64) -> String {
    if bytes == 0 {
        "0 B".into()
    } else {
        theme::format_size(bytes)
    }
}
