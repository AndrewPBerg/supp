/// A trait for types that can validate their own state.
pub trait Validate {
    /// Returns `Ok(())` if valid, or an error message.
    fn validate(&self) -> Result<(), String>;
}

/// A trait for types that serialize to a key-value map.
pub trait ToMap {
    fn to_map(&self) -> Vec<(String, String)>;
}

/// A user account with role-based access.
#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub name: String,
    pub email: String,
    pub role: Role,
}

/// Permission levels.
#[derive(Debug, Clone, PartialEq)]
pub enum Role {
    Viewer,
    Editor,
    Admin,
}

impl Validate for User {
    fn validate(&self) -> Result<(), String> {
        if self.email.is_empty() {
            return Err("email is required".into());
        }
        if !self.email.contains('@') {
            return Err("invalid email".into());
        }
        Ok(())
    }
}

impl ToMap for User {
    fn to_map(&self) -> Vec<(String, String)> {
        vec![
            ("id".into(), self.id.clone()),
            ("name".into(), self.name.clone()),
            ("email".into(), self.email.clone()),
            ("role".into(), format!("{:?}", self.role)),
        ]
    }
}

impl User {
    /// Check whether this user has admin privileges.
    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
    }
}

/// A project belonging to a user.
#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub owner_id: String,
    pub tags: Vec<String>,
}

impl Validate for Project {
    fn validate(&self) -> Result<(), String> {
        if self.title.is_empty() {
            return Err("title is required".into());
        }
        Ok(())
    }
}

/// A task within a project.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: String,
    pub project_id: String,
    pub description: String,
    pub done: bool,
}

impl Task {
    /// Toggle the completion status.
    pub fn toggle(&mut self) {
        self.done = !self.done;
    }
}
