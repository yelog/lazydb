use thiserror::Error;
use uuid::Uuid;

use crate::profile::{ConnectionGroup, ConnectionGroupNameError, ProfileCollection};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoveDirection {
    Up,
    Down,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum OrganizationError {
    #[error("connection group {0} was not found")]
    GroupNotFound(Uuid),
    #[error("connection profile {0} was not found")]
    ProfileNotFound(Uuid),
    #[error("connection group name `{0}` already exists")]
    DuplicateGroupName(String),
    #[error(transparent)]
    InvalidGroupName(#[from] ConnectionGroupNameError),
}

pub fn create_group(
    collection: &mut ProfileCollection,
    id: Uuid,
    name: String,
) -> Result<(), OrganizationError> {
    let group = ConnectionGroup::new(id, name)?;
    if collection
        .groups
        .iter()
        .any(|candidate| candidate.normalized_name() == group.normalized_name())
    {
        return Err(OrganizationError::DuplicateGroupName(group.name));
    }
    collection.groups.push(group);
    Ok(())
}

pub fn rename_group(
    collection: &mut ProfileCollection,
    id: Uuid,
    name: String,
) -> Result<(), OrganizationError> {
    let replacement = ConnectionGroup::new(id, name)?;
    if collection
        .groups
        .iter()
        .any(|group| group.id != id && group.normalized_name() == replacement.normalized_name())
    {
        return Err(OrganizationError::DuplicateGroupName(replacement.name));
    }
    let group = collection
        .groups
        .iter_mut()
        .find(|group| group.id == id)
        .ok_or(OrganizationError::GroupNotFound(id))?;
    group.name = replacement.name;
    Ok(())
}

pub fn delete_group(
    collection: &mut ProfileCollection,
    id: Uuid,
) -> Result<usize, OrganizationError> {
    let index = collection
        .groups
        .iter()
        .position(|group| group.id == id)
        .ok_or(OrganizationError::GroupNotFound(id))?;
    collection.groups.remove(index);
    let mut removed = 0;
    for profile in &mut collection.profiles {
        if profile.group_id == Some(id) {
            profile.group_id = None;
            removed += 1;
        }
    }
    Ok(removed)
}

pub fn assign_profile(
    collection: &mut ProfileCollection,
    profile_id: Uuid,
    group_id: Option<Uuid>,
) -> Result<(), OrganizationError> {
    if let Some(group_id) = group_id
        && !collection.groups.iter().any(|group| group.id == group_id)
    {
        return Err(OrganizationError::GroupNotFound(group_id));
    }
    let profile = collection
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or(OrganizationError::ProfileNotFound(profile_id))?;
    profile.group_id = group_id;
    Ok(())
}

pub fn move_profile(
    collection: &mut ProfileCollection,
    profile_id: Uuid,
    sibling_ids: &[Uuid],
    direction: MoveDirection,
) -> Result<bool, OrganizationError> {
    if !collection
        .profiles
        .iter()
        .any(|profile| profile.id == profile_id)
    {
        return Err(OrganizationError::ProfileNotFound(profile_id));
    }
    let visible_index = sibling_ids.iter().position(|id| *id == profile_id);
    let Some(visible_index) = visible_index else {
        return Ok(false);
    };
    let target = match direction {
        MoveDirection::Up => visible_index.checked_sub(1),
        MoveDirection::Down => (visible_index + 1 < sibling_ids.len()).then_some(visible_index + 1),
    };
    let Some(target) = target else {
        return Ok(false);
    };
    let first = collection
        .profiles
        .iter()
        .position(|p| p.id == profile_id)
        .unwrap();
    let second = collection
        .profiles
        .iter()
        .position(|p| p.id == sibling_ids[target])
        .unwrap();
    collection.profiles.swap(first, second);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::import_connection_url;

    fn collection() -> ProfileCollection {
        let mut first = import_connection_url(":memory:", Some("first"))
            .unwrap()
            .profile;
        first.id = Uuid::from_u128(10);
        let mut second = import_connection_url(":memory:", Some("second"))
            .unwrap()
            .profile;
        second.id = Uuid::from_u128(11);
        ProfileCollection {
            groups: Vec::new(),
            profiles: vec![first, second],
        }
    }

    #[test]
    fn group_crud_and_assignment_are_atomic_on_validation_errors() {
        let mut collection = collection();
        create_group(&mut collection, Uuid::from_u128(1), " Production ".into()).unwrap();
        assert_eq!(collection.groups[0].name, "Production");
        assert_eq!(
            create_group(&mut collection, Uuid::from_u128(2), "production".into()),
            Err(OrganizationError::DuplicateGroupName("production".into()))
        );
        assign_profile(
            &mut collection,
            Uuid::from_u128(10),
            Some(Uuid::from_u128(1)),
        )
        .unwrap();
        assert_eq!(delete_group(&mut collection, Uuid::from_u128(1)), Ok(1));
        assert_eq!(collection.profiles[0].group_id, None);
    }

    #[test]
    fn moving_visible_siblings_preserves_other_order_and_boundaries_are_noops() {
        let mut collection = collection();
        assert!(
            !move_profile(
                &mut collection,
                Uuid::from_u128(10),
                &[Uuid::from_u128(10), Uuid::from_u128(11)],
                MoveDirection::Up
            )
            .unwrap()
        );
        assert!(
            move_profile(
                &mut collection,
                Uuid::from_u128(10),
                &[Uuid::from_u128(10), Uuid::from_u128(11)],
                MoveDirection::Down
            )
            .unwrap()
        );
        assert_eq!(
            collection.profiles.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec![Uuid::from_u128(11), Uuid::from_u128(10)]
        );
    }
}
