//! Ops: the one way every edit travels. A user action becomes a patch of the fields it changed
//! on one entity; the app applies it locally and queues it, the server applies it in arrival
//! order and hands it to everyone else. Field-level last-write-wins falls out of that: patches
//! to different fields or entities never clash, the same field resolves to the latest patch.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::position;

pub const MAX_NAME_CHARS: usize = 255;
pub const MAX_TEXT_CHARS: usize = 10_000;
/// Longest cue: one week.
pub const MAX_DURATION_SECONDS: i64 = 7 * 24 * 3600;

pub type Fields = Map<String, Value>;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Show,
    Role,
    Line,
    Comment,
}

impl EntityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Show => "show",
            Self::Role => "role",
            Self::Line => "line",
            Self::Comment => "comment",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "show" => Some(Self::Show),
            "role" => Some(Self::Role),
            "line" => Some(Self::Line),
            "comment" => Some(Self::Comment),
            _ => None,
        }
    }

    /// Fields a patch may set on this entity.
    pub fn fields(self) -> &'static [&'static str] {
        match self {
            Self::Show => &["name", "time"],
            Self::Role => &["name"],
            Self::Line => &["name", "note", "duration", "absolute_time", "position"],
            Self::Comment => &["line_id", "role_id", "comment"],
        }
    }

    /// Fields a creating patch must carry (a line also needs `position` or a `place`).
    pub fn required_on_create(self) -> &'static [&'static str] {
        match self {
            Self::Show => &["name", "time"],
            Self::Role => &["name"],
            Self::Line => &["name", "note", "duration"],
            Self::Comment => &["line_id", "comment"],
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpKind {
    Set,
    Delete,
}

/// "Put this line right after `after`" (`None`: at the very start). Whoever applies the op
/// turns it into a concrete `position`; clients that cannot compute keys (the browser) send
/// this instead.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub after: Option<Uuid>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Op {
    /// Idempotency key: the server applies an op id once, however often it is sent.
    pub op_id: Uuid,
    pub entity: EntityKind,
    pub id: Uuid,
    pub kind: OpKind,
    #[serde(default)]
    pub fields: Fields,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<Place>,
}

/// An applied op as the server recorded it (`place` resolved into a concrete `position`),
/// numbered per show. What clients pull and apply.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Change {
    pub rev: i64,
    pub op_id: Uuid,
    pub entity: EntityKind,
    pub id: Uuid,
    pub kind: OpKind,
    #[serde(default)]
    pub fields: Fields,
    pub author_id: Option<Uuid>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SyncError {
    UnknownField { entity: EntityKind, field: String },
    InvalidField { field: &'static str, reason: &'static str },
    MissingField(&'static str),
    DeleteWithFields,
    PlaceOnlyForLines,
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownField { entity, field } => write!(f, "{} has no field {field:?}", entity.as_str()),
            Self::InvalidField { field, reason } => write!(f, "invalid {field}: {reason}"),
            Self::MissingField(field) => write!(f, "missing required field {field}"),
            Self::DeleteWithFields => write!(f, "a delete carries no fields"),
            Self::PlaceOnlyForLines => write!(f, "only lines can be placed"),
        }
    }
}

impl std::error::Error for SyncError {}

fn text(value: &Value, field: &'static str, max: usize, allow_empty: bool) -> Result<(), SyncError> {
    let Some(string) = value.as_str() else {
        return Err(SyncError::InvalidField { field, reason: "must be text" });
    };
    if (!allow_empty && string.trim().is_empty()) || string.chars().count() > max {
        return Err(SyncError::InvalidField { field, reason: "empty or too long" });
    }
    Ok(())
}

fn uuid(value: &Value, field: &'static str, nullable: bool) -> Result<(), SyncError> {
    match value {
        Value::Null if nullable => Ok(()),
        Value::String(s) if Uuid::parse_str(s).is_ok() => Ok(()),
        _ => Err(SyncError::InvalidField { field, reason: "must be an id" }),
    }
}

fn integer(value: &Value, field: &'static str, range: std::ops::RangeInclusive<i64>) -> Result<(), SyncError> {
    match value.as_i64() {
        Some(n) if range.contains(&n) => Ok(()),
        _ => Err(SyncError::InvalidField { field, reason: "must be a whole number in range" }),
    }
}

impl Op {
    /// Checks the shape of the patch: known fields, right types and sizes. Whether the entity
    /// exists, belongs to this show, or may be edited by this user is the applier's business.
    pub fn validate(&self) -> Result<(), SyncError> {
        if self.kind == OpKind::Delete {
            return if self.fields.is_empty() && self.place.is_none() { Ok(()) } else { Err(SyncError::DeleteWithFields) };
        }
        if self.place.is_some() && self.entity != EntityKind::Line {
            return Err(SyncError::PlaceOnlyForLines);
        }
        for (field, value) in &self.fields {
            match (self.entity, field.as_str()) {
                (EntityKind::Show, "name") => text(value, "name", MAX_NAME_CHARS, false)?,
                (EntityKind::Show, "time") => integer(value, "time", 0..=i64::MAX / 2)?,
                (EntityKind::Role, "name") => text(value, "name", MAX_NAME_CHARS, false)?,
                (EntityKind::Line, "name") => text(value, "name", MAX_NAME_CHARS, false)?,
                (EntityKind::Line, "note") => text(value, "note", MAX_TEXT_CHARS, true)?,
                (EntityKind::Line, "duration") => integer(value, "duration", 0..=MAX_DURATION_SECONDS)?,
                (EntityKind::Line, "absolute_time") if value.is_null() => {}
                (EntityKind::Line, "absolute_time") => integer(value, "absolute_time", 0..=i64::MAX / 2)?,
                (EntityKind::Line, "position") => {
                    if !value.as_str().is_some_and(position::is_valid_key) {
                        return Err(SyncError::InvalidField { field: "position", reason: "not a position key" });
                    }
                }
                (EntityKind::Comment, "line_id") => uuid(value, "line_id", false)?,
                (EntityKind::Comment, "role_id") => uuid(value, "role_id", true)?,
                (EntityKind::Comment, "comment") => text(value, "comment", MAX_TEXT_CHARS, false)?,
                _ => return Err(SyncError::UnknownField { entity: self.entity, field: field.clone() }),
            }
        }
        Ok(())
    }

    /// `validate`, plus everything an entity needs to come into existence.
    pub fn validate_create(&self) -> Result<(), SyncError> {
        self.validate()?;
        if self.kind == OpKind::Delete {
            return Err(SyncError::MissingField("fields"));
        }
        for field in self.entity.required_on_create() {
            if !self.fields.contains_key(*field) {
                return Err(SyncError::MissingField(field));
            }
        }
        if self.entity == EntityKind::Line && !self.fields.contains_key("position") && self.place.is_none() {
            return Err(SyncError::MissingField("position"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn op(entity: EntityKind, kind: OpKind, fields: Value) -> Op {
        Op {
            op_id: Uuid::new_v4(),
            entity,
            id: Uuid::new_v4(),
            kind,
            fields: fields.as_object().cloned().unwrap_or_default(),
            place: None,
        }
    }

    #[test]
    fn every_listed_field_validates_and_unknown_ones_do_not() {
        let line = op(EntityKind::Line, OpKind::Set, json!({
            "name": "Intro", "note": "", "duration": 90, "absolute_time": null, "position": "V"
        }));
        assert_eq!(line.validate(), Ok(()));
        assert_eq!(line.validate_create(), Ok(()));
        for entity in [EntityKind::Show, EntityKind::Role, EntityKind::Line, EntityKind::Comment] {
            let unknown = op(entity, OpKind::Set, json!({ "bogus": 1 }));
            assert!(matches!(unknown.validate(), Err(SyncError::UnknownField { .. })));
        }
        // Fields of another entity are unknown too (no smuggling `line_id` onto a show).
        assert!(op(EntityKind::Show, OpKind::Set, json!({"line_id": Uuid::new_v4()})).validate().is_err());
        assert!(op(EntityKind::Show, OpKind::Set, json!({"time": "noon"})).validate().is_err());
    }

    #[test]
    fn creating_needs_the_required_fields() {
        let partial = op(EntityKind::Show, OpKind::Set, json!({"name": "Gala"}));
        assert_eq!(partial.validate(), Ok(()), "a patch may be partial");
        assert_eq!(partial.validate_create(), Err(SyncError::MissingField("time")));

        let mut line = op(EntityKind::Line, OpKind::Set, json!({"name": "Cue", "note": "", "duration": 5}));
        assert_eq!(line.validate_create(), Err(SyncError::MissingField("position")));
        line.place = Some(Place { after: None });
        assert_eq!(line.validate_create(), Ok(()));
        assert_eq!(op(EntityKind::Show, OpKind::Delete, json!({})).validate_create(), Err(SyncError::MissingField("fields")));
    }

    #[test]
    fn limits_are_enforced() {
        let long = "x".repeat(MAX_NAME_CHARS + 1);
        assert!(op(EntityKind::Role, OpKind::Set, json!({"name": long})).validate().is_err());
        assert!(op(EntityKind::Role, OpKind::Set, json!({"name": "   "})).validate().is_err());
        assert!(op(EntityKind::Line, OpKind::Set, json!({"duration": -1})).validate().is_err());
        assert!(op(EntityKind::Line, OpKind::Set, json!({"duration": MAX_DURATION_SECONDS + 1})).validate().is_err());
        assert!(op(EntityKind::Line, OpKind::Set, json!({"position": "a0"})).validate().is_err());
        assert!(op(EntityKind::Comment, OpKind::Set, json!({"line_id": "nope"})).validate().is_err());
        assert!(op(EntityKind::Comment, OpKind::Set, json!({"role_id": null})).validate().is_ok());
    }

    #[test]
    fn deletes_and_places_are_checked() {
        assert!(op(EntityKind::Line, OpKind::Delete, json!({})).validate().is_ok());
        assert_eq!(op(EntityKind::Line, OpKind::Delete, json!({"name": "x"})).validate(), Err(SyncError::DeleteWithFields));
        let mut role = op(EntityKind::Role, OpKind::Set, json!({"name": "Audio"}));
        role.place = Some(Place { after: None });
        assert_eq!(role.validate(), Err(SyncError::PlaceOnlyForLines));
    }

    #[test]
    fn ops_round_trip_through_json_in_the_wire_shape() {
        let mut line = op(EntityKind::Line, OpKind::Set, json!({"name": "Cue"}));
        line.place = Some(Place { after: Some(Uuid::nil()) });
        let json = serde_json::to_value(&line).unwrap();
        assert_eq!(json["entity"], "line");
        assert_eq!(json["kind"], "set");
        assert_eq!(json["place"]["after"], Uuid::nil().to_string());
        assert_eq!(serde_json::from_value::<Op>(json).unwrap(), line);
        let bare: Op = serde_json::from_value(json!({
            "op_id": Uuid::nil(), "entity": "show", "id": Uuid::nil(), "kind": "delete"
        })).unwrap();
        assert!(bare.fields.is_empty() && bare.place.is_none());
    }
}
