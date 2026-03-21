use crate::models::{Project, Role, Task, User, Validate};

/// In-memory store holding all users and projects.
pub struct AppState {
    users: Vec<User>,
    projects: Vec<Project>,
    next_id: usize,
}

impl AppState {
    /// Create an empty application state.
    pub fn new() -> Self {
        Self {
            users: Vec::new(),
            projects: Vec::new(),
            next_id: 1,
        }
    }

    fn gen_id(&mut self, prefix: &str) -> String {
        let id = format!("{}-{}", prefix, self.next_id);
        self.next_id += 1;
        id
    }

    /// Create and store a new user.
    ///
    /// Returns an error if validation fails.
    pub fn create_user(
        &mut self,
        name: &str,
        email: &str,
        role: Role,
    ) -> Result<&User, String> {
        let id = self.gen_id("u");
        let user = User {
            id,
            name: name.to_string(),
            email: email.to_string(),
            role,
        };
        user.validate()?;
        self.users.push(user);
        Ok(self.users.last().unwrap())
    }

    /// Look up a user by ID.
    pub fn get_user(&self, id: &str) -> Option<&User> {
        self.users.iter().find(|u| u.id == id)
    }

    /// Create a project owned by the given user.
    pub fn create_project(&mut self, title: &str, owner: &User) -> Result<&Project, String> {
        let id = self.gen_id("p");
        let project = Project {
            id,
            title: title.to_string(),
            owner_id: owner.id.clone(),
            tags: Vec::new(),
        };
        project.validate()?;
        self.projects.push(project);
        Ok(self.projects.last().unwrap())
    }

    /// Build a task for a project (not stored in AppState).
    pub fn build_task(&mut self, project: &Project, description: &str) -> Task {
        let id = self.gen_id("t");
        Task {
            id,
            project_id: project.id.clone(),
            description: description.to_string(),
            done: false,
        }
    }

    /// Return all admin users.
    pub fn admin_users(&self) -> Vec<&User> {
        self.users.iter().filter(|u| u.is_admin()).collect()
    }
}
