//! One compact inline footprint panel for every package source.

use gpui_kit::prelude::*;
use gpui_kit::{div, px, AnyElement};

use crate::domain::listing;
use crate::domain::package::{InstalledPackage, PackageSource};
use crate::domain::package_details::PackageDetails;
use crate::theme;

use super::widgets::plan_rows;

pub(super) fn detail_element(
    pkg: &InstalledPackage,
    details: Option<&PackageDetails>,
) -> AnyElement {
    div()
        .px(px(20.))
        .pb(px(20.))
        .pt(px(12.))
        .child(super::package_details::breakdown(details, pkg.source))
        .when(pkg.source == PackageSource::Steam, |this| {
            this.child(plan_rows(vec![
                ("AppID", pkg.package_id.clone()),
                (
                    "Steam update",
                    theme::steam_update_status_label(pkg.steam_update_status).into(),
                ),
            ]))
            .child(
                div()
                    .pt(px(6.))
                    .text_size(px(12.))
                    .line_height(px(18.))
                    .text_color(theme::text_dim())
                    .child("Steam's last recorded status; open Steam to check for newer updates."),
            )
        })
        .when_some(listing::uninstall_block_reason(pkg), |this, reason| {
            this.child(
                div()
                    .pt(px(12.))
                    .text_size(px(12.))
                    .text_color(theme::text_dim())
                    .child(reason),
            )
        })
        .into_any_element()
}
