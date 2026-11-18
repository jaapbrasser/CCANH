use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// A group in the hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    /// `null` for a top-level group.
    pub parent_id: Option<Uuid>,
    /// Cameras directly in this group (not counting subgroups).
    pub camera_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

/// A group or tag as listed on a camera.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct GroupRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TagRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Tag {
    pub id: Uuid,
    pub name: String,
    pub camera_count: i64,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[schema(value_type = String, format = DateTime)]
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct NewGroup {
    pub name: String,
    /// Omit or `null` for a top-level group.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
}

/// Rename and/or move a group. Omitted fields are unchanged; `parent_id: null`
/// moves the group to the top level.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct GroupPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    pub parent_id: Option<Option<Uuid>>,
}

/// Distinguish an explicit `null` (Some(None)) from an omitted field (None).
fn present<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// What happens to a deleted group's subgroups and cameras.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeleteMode {
    /// Subgroups and cameras move to the deleted group's parent. For a
    /// top-level group, subgroups become top-level and cameras are detached.
    MoveUp,
    /// Subgroups become top-level groups; cameras leave the group.
    Detach,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DeleteGroupQuery {
    /// Required: what happens to subgroups and cameras.
    #[param(inline)]
    pub mode: DeleteMode,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TagInput {
    pub name: String,
}
