use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConnectionIdentity {
    pub profile_id: Uuid,
    pub generation: u64,
}
