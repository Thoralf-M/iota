// Copyright (c) Mysten Labs, Inc.
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use diesel::prelude::*;
use iota_types::display::DisplayVersionUpdatedEvent;

use crate::schema::display;

#[derive(Queryable, Insertable, Selectable, Debug, Clone)]
#[diesel(table_name = display)]
pub struct StoredDisplay {
    pub object_type: String,
    pub id: Vec<u8>,
    pub version: i16,
    pub bcs: Vec<u8>,
}

impl StoredDisplay {
    pub fn try_from_event(event: &iota_types::event::Event) -> Option<Self> {
        let (ty, display_event) = DisplayVersionUpdatedEvent::try_from_event(event)?;

        Some(Self {
            object_type: ty.to_canonical_string(/* with_prefix */ true),
            id: display_event.id.bytes.to_vec(),
            version: display_event.version as i16,
            bcs: event.contents.clone(),
        })
    }

    pub fn to_display_update_event(&self) -> Result<DisplayVersionUpdatedEvent, bcs::Error> {
        bcs::from_bytes(&self.bcs)
    }
}
