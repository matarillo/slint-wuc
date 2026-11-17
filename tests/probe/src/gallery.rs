// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: MIT

//! Shows upstream's widget gallery (`examples/gallery`), with the table data its `main.rs` sets.

use slint::{Model, ModelExt, ModelRc, SharedString, StandardListViewItem, VecModel};
use std::rc::Rc;

include!(concat!(env!("OUT_DIR"), "/gallery.rs"));

fn main() {
    let app = App::new().unwrap();

    let row_data: Rc<VecModel<ModelRc<StandardListViewItem>>> = Rc::new(VecModel::default());
    for r in 1..101 {
        let items = Rc::new(VecModel::default());
        for c in 1..5 {
            items.push(slint::format!("Item {r}.{c}").into());
        }
        row_data.push(items.into());
    }
    app.global::<TableViewPageAdapter>().set_row_data(row_data.into());
    app.global::<TableViewPageAdapter>().on_filter_sort_model(filter_sort_model);

    app.run().unwrap();
}

fn filter_sort_model(
    source_model: ModelRc<ModelRc<StandardListViewItem>>,
    filter: SharedString,
    sort_index: i32,
    sort_ascending: bool,
) -> ModelRc<ModelRc<StandardListViewItem>> {
    let mut model = source_model.clone();
    if !filter.is_empty() {
        let filter = filter.to_lowercase();
        model = Rc::new(source_model.filter(move |e| {
            e.row_data(0).unwrap().text.to_lowercase().contains(filter.as_str())
        }))
        .into();
    }
    if sort_index >= 0 {
        model = Rc::new(model.clone().sort_by(move |r_a, r_b| {
            let c_a = r_a.row_data(sort_index as usize).unwrap();
            let c_b = r_b.row_data(sort_index as usize).unwrap();
            if sort_ascending { c_a.text.cmp(&c_b.text) } else { c_b.text.cmp(&c_a.text) }
        }))
        .into();
    }
    model
}
