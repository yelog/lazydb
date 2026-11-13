use mongodb::{
    Client,
    bson::{Bson, Document, doc},
};
use secrecy::{ExposeSecret, SecretString};

use crate::{
    db::{DatabaseError, ErrorCategory},
    security::RedactedSecret,
};

#[derive(Clone, Debug, PartialEq)]
pub struct MongoPrincipalInfo {
    pub name: String,
    pub database: String,
    pub roles: Vec<Document>,
    pub inherited_roles: Vec<Document>,
    pub privileges: Vec<Document>,
    pub authentication_restrictions: Vec<Document>,
}

pub async fn users(
    client: &Client,
    database: &str,
) -> mongodb::error::Result<Vec<MongoPrincipalInfo>> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.database(database).run_command(
            doc! {"usersInfo": 1, "showPrivileges": true, "showAuthenticationRestrictions": false},
        ),
    )
    .await
    .map_err(|_| mongodb::error::Error::custom("usersInfo request timed out"))??;
    let users = response.get_array("users").cloned().unwrap_or_default();
    Ok(users
        .into_iter()
        .filter_map(|value| {
            let user = value.as_document()?;
            Some(MongoPrincipalInfo {
                name: user.get_str("user").ok()?.to_owned(),
                database: user.get_str("db").ok()?.to_owned(),
                roles: user
                    .get_array("roles")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|role| role.as_document().cloned())
                    .collect(),
                inherited_roles: user
                    .get_array("inheritedRoles")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|role| role.as_document().cloned())
                    .collect(),
                privileges: user
                    .get_array("inheritedPrivileges")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_document().cloned())
                    .collect(),
                authentication_restrictions: user
                    .get_array("inheritedAuthenticationRestrictions")
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_document().cloned())
                    .collect(),
            })
        })
        .collect())
}

pub async fn users_for_adapter(
    adapter: &super::MongoAdapter,
    database: &str,
) -> mongodb::error::Result<Vec<MongoPrincipalInfo>> {
    users(checked_client(adapter)?, database).await
}

pub async fn roles(
    adapter: &super::MongoAdapter,
    database: &str,
) -> mongodb::error::Result<Vec<Document>> {
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        checked_client(adapter)?.database(database).run_command(
            doc! {"rolesInfo": 1, "showPrivileges": true, "showAuthenticationRestrictions": false},
        ),
    )
    .await
    .map_err(|_| mongodb::error::Error::custom("rolesInfo request timed out"))??;
    Ok(response
        .get_array("roles")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|role| role.as_document().cloned())
        .take(500)
        .collect())
}

fn checked_client(adapter: &super::MongoAdapter) -> mongodb::error::Result<&mongodb::Client> {
    adapter
        .checked_client()
        .map_err(|error| mongodb::error::Error::custom(error.to_string()))
}

pub fn validate_management_command(command: &Document, database: &str) -> Result<(), &'static str> {
    let name = command
        .keys()
        .next()
        .ok_or("Mongo principal command is empty")?;
    if !matches!(name.as_str(), "usersInfo" | "rolesInfo") {
        return Err("Unsupported Mongo principal command");
    }
    if command.contains_key("$db") {
        return Err("Mongo principal command cannot override its database");
    }
    if database.is_empty() {
        return Err("Mongo principal database is required");
    }
    Ok(())
}

#[derive(Clone)]
pub struct MongoPrincipalMutation {
    pub database: String,
    command: Document,
    password: Option<RedactedSecret>,
}

impl MongoPrincipalMutation {
    pub fn targets_database(&self, selected_database: &str) -> bool {
        self.database == selected_database
    }

    pub fn confirmation_target(&self) -> String {
        let target = self.target_name().unwrap_or_default();
        if target.contains('@') {
            target.to_owned()
        } else {
            format!("{}:{target}", self.database)
        }
    }
}

impl MongoPrincipalMutation {
    pub fn new(
        database: impl Into<String>,
        command_name: &str,
        target: impl Into<String>,
        password: Option<SecretString>,
        roles: Vec<Document>,
        privileges: Vec<Document>,
    ) -> Result<Self, &'static str> {
        let database = database.into();
        if database.is_empty() || database.contains('.') || database.contains('\0') {
            return Err("Mongo principal database is invalid");
        }
        if database != "admin" && matches!(command_name, "createRole" | "updateRole" | "dropRole") {
            return Err("Custom role management requires the admin database");
        }
        if database != "admin"
            && matches!(
                command_name,
                "grantPrivilegesToRole" | "revokePrivilegesFromRole"
            )
        {
            return Err("Custom role privilege management requires the admin database");
        }
        let target = target.into();
        if target.trim().is_empty() || target.contains('\0') {
            return Err("Mongo principal name is required");
        }
        if matches!(command_name, "createUser" | "updateUser") && password.is_none() {
            return Err("An ephemeral password is required for this operation");
        }
        if command_name == "createUser" && roles.is_empty() {
            return Err("createUser requires explicit roles");
        }
        if !matches!(command_name, "createUser" | "updateUser") && password.is_some() {
            return Err("Password is only valid for createUser or updateUser");
        }
        let (_, command) = match command_name {
            "createUser" | "updateUser" | "dropUser" => {
                let mut command = principal_command(command_name, &target);
                if command_name == "dropUser" && !roles.is_empty() {
                    return Err("dropUser does not accept roles");
                }
                if !privileges.is_empty() {
                    return Err("Privileges are not valid for a user operation");
                }
                if !roles.is_empty() {
                    command.insert("roles", roles);
                }
                (command_name, command)
            }
            "createRole" | "updateRole" => {
                if privileges.is_empty() {
                    return Err(
                        "Role creation and update require privileges and do not accept roles",
                    );
                }
                let mut command = principal_command(command_name, &target);
                command.insert("privileges", privileges);
                command.insert(
                    "roles",
                    roles
                        .into_iter()
                        .map(|mut role| {
                            if !role.contains_key("db") {
                                role.insert("db", &database);
                            }
                            role
                        })
                        .collect::<Vec<_>>(),
                );
                (command_name, command)
            }
            "dropRole" => {
                if !roles.is_empty() || !privileges.is_empty() {
                    return Err("dropRole does not accept roles or privileges");
                }
                (command_name, principal_command(command_name, &target))
            }
            "grantRolesToUser" | "revokeRolesFromUser" => {
                if roles.is_empty() {
                    return Err("At least one role is required");
                }
                let mut command = principal_command(command_name, &target);
                command.insert(
                    "roles",
                    roles
                        .into_iter()
                        .map(|mut role| {
                            if !role.contains_key("db") {
                                role.insert("db", &database);
                            }
                            role
                        })
                        .collect::<Vec<_>>(),
                );
                (command_name, command)
            }
            "grantPrivilegesToRole" | "revokePrivilegesFromRole" => {
                if privileges.is_empty() {
                    return Err("At least one privilege is required");
                }
                let mut command = principal_command(command_name, &target);
                command.insert("privileges", privileges);
                (command_name, command)
            }
            _ => return Err("Unsupported Mongo principal mutation"),
        };
        Ok(Self {
            database,
            command,
            password: password.map(|secret| RedactedSecret::new(secret.expose_secret())),
        })
    }

    pub fn has_password(&self) -> bool {
        self.password
            .as_ref()
            .is_some_and(|secret| !secret.is_empty())
    }

    pub fn command_name(&self) -> Option<&str> {
        self.command.keys().next().map(String::as_str)
    }

    pub fn command_contains_field(&self, field: &str) -> bool {
        self.command.contains_key(field)
    }

    pub fn target_name(&self) -> Option<&str> {
        self.command.values().next().and_then(Bson::as_str)
    }

    pub async fn execute_for_database(
        &self,
        client: &Client,
        selected_database: &str,
        read_only: bool,
    ) -> Result<(), DatabaseError> {
        if read_only {
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("mongodb_principal_read_only".into()),
                message: "MongoDB principal writes are disabled for this profile".into(),
                diagnostic: None,
            });
        }
        if !self.targets_database(selected_database) {
            return Err(DatabaseError {
                category: ErrorCategory::Unsupported,
                code: Some("mongodb_principal_scope".into()),
                message: "MongoDB principal operation is outside the selected database".into(),
                diagnostic: None,
            });
        }
        self.execute(client).await
    }

    pub async fn execute(&self, client: &Client) -> Result<(), DatabaseError> {
        let mut command = self.command.clone();
        if let Some(password) = self.password.as_ref() {
            command.insert("pwd", Bson::String(password.expose().to_owned()));
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.database(&self.database).run_command(command),
        )
        .await;
        match result {
            Err(_) => Err(DatabaseError {
                category: ErrorCategory::Network,
                code: Some("mongodb_principal_timeout".into()),
                message: "MongoDB principal operation timed out; verify server state before retrying".into(),
                diagnostic: None,
            }),
            Ok(Err(_)) => Err(DatabaseError {
                category: ErrorCategory::Permission,
                code: Some("mongodb_principal_operation_failed".into()),
                message: "MongoDB principal operation failed; details were withheld to protect credentials".into(),
                diagnostic: None,
            }),
            Ok(Ok(_)) => Ok(()),
        }
    }
}

impl std::fmt::Debug for MongoPrincipalMutation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MongoPrincipalMutation")
            .field("database", &self.database)
            .field("command", &"<redacted principal command>")
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

impl PartialEq for MongoPrincipalMutation {
    fn eq(&self, other: &Self) -> bool {
        self.database == other.database
            && self.command == other.command
            && self.password == other.password
    }
}

impl Eq for MongoPrincipalMutation {}

fn principal_command(name: &str, target: &str) -> Document {
    let mut command = Document::new();
    command.insert(name, target);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::SecretString;

    #[test]
    fn principal_commands_are_database_scoped_and_password_safe() {
        assert!(validate_management_command(&doc! {"usersInfo": 1}, "app").is_ok());
        assert!(
            validate_management_command(&doc! {"usersInfo": 1, "$db": "admin"}, "app").is_err()
        );
        assert!(
            validate_management_command(
                &doc! {"createUser": {"user": "u", "pwd": "secret"}},
                "app"
            )
            .is_err()
        );
        assert!(validate_management_command(&doc! {"dropUser": "reader"}, "app").is_err());
    }

    #[test]
    fn principal_mutation_keeps_password_out_of_debug_and_stored_command() {
        let mutation = MongoPrincipalMutation::new(
            "app",
            "createUser",
            "reader",
            Some(SecretString::from("do-not-log-this")),
            vec![doc! { "role": "read", "db": "app" }],
            Vec::new(),
        )
        .unwrap();
        let update = MongoPrincipalMutation::new(
            "app",
            "updateUser",
            "reader",
            Some(SecretString::from("replacement-secret")),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert!(!format!("{update:?}").contains("replacement-secret"));
        assert_eq!(mutation.command_name(), Some("createUser"));
        assert!(!mutation.command_contains_field("pwd"));
        assert!(mutation.command_contains_field("roles"));
        assert!(mutation.has_password());
        let debug = format!("{mutation:?}");
        assert!(!debug.contains("do-not-log-this"));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn principal_mutations_require_typed_roles_and_reject_invalid_scope() {
        assert!(
            MongoPrincipalMutation::new(
                "app",
                "grantRolesToUser",
                "reader",
                None,
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            MongoPrincipalMutation::new(
                "app.admin",
                "dropUser",
                "reader",
                None,
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            MongoPrincipalMutation::new("app", "arbitrary", "reader", None, Vec::new(), Vec::new())
                .is_err()
        );
        assert!(
            MongoPrincipalMutation::new(
                "admin",
                "createRole",
                "reader",
                None,
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            MongoPrincipalMutation::new(
                "app",
                "createUser",
                "reader",
                None,
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        assert!(
            MongoPrincipalMutation::new(
                "app",
                "dropUser",
                "reader",
                Some(SecretString::from("x")),
                Vec::new(),
                Vec::new()
            )
            .is_err()
        );
        let role = MongoPrincipalMutation::new(
            "admin",
            "createRole",
            "reader",
            None,
            vec![doc! { "role": "read", "db": "app" }],
            vec![doc! { "resource": { "db": "app", "collection": "" }, "actions": ["find"] }],
        )
        .unwrap();
        assert_eq!(role.command_name(), Some("createRole"));
        assert!(role.command_contains_field("privileges"));
        assert!(role.command_contains_field("roles"));
        assert!(
            MongoPrincipalMutation::new(
                "app",
                "createRole",
                "reader",
                None,
                Vec::new(),
                vec![doc! { "resource": { "db": "app", "collection": "" }, "actions": ["find"] }]
            )
            .is_err()
        );
    }

    #[test]
    fn mutation_database_must_match_selected_database() {
        let mutation =
            MongoPrincipalMutation::new("app", "dropUser", "reader", None, Vec::new(), Vec::new())
                .unwrap();
        assert!(mutation.targets_database("app"));
        assert!(!mutation.targets_database("admin"));
    }
}
