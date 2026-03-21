import { UserData, ProjectData, TaskData, Role, EntityId } from "./models";
import { ValidatedStore } from "./store";

/** Store with email-uniqueness validation. */
class UserStore extends ValidatedStore<UserData> {
  validate(item: UserData): string | null {
    if (!item.email.includes("@")) return "invalid email";
    const dup = this.all().find((u) => u.email === item.email && u.id !== item.id);
    if (dup) return "email already taken";
    return null;
  }
}

/** Store with title-required validation. */
class ProjectStore extends ValidatedStore<ProjectData> {
  validate(item: ProjectData): string | null {
    if (!item.title.trim()) return "title is required";
    return null;
  }
}

const users = new UserStore();
const projects = new ProjectStore();

let nextId = 1;
function genId(prefix: string): EntityId {
  return `${prefix}-${nextId++}`;
}

/** Create and store a new user. */
export function createUser(name: string, email: string, role: Role = "viewer"): UserData {
  const user: UserData = {
    id: genId("u"),
    createdAt: new Date(),
    name,
    email,
    role,
  };
  users.add(user);
  return user;
}

/** Create a project owned by the given user. */
export function createProject(title: string, owner: UserData): ProjectData {
  const project: ProjectData = {
    id: genId("p"),
    createdAt: new Date(),
    title,
    ownerId: owner.id,
    tags: [],
  };
  projects.add(project);
  return project;
}

/** Build a task object (not stored — belongs to a project). */
export function buildTask(projectId: EntityId, description: string): TaskData {
  return {
    id: genId("t"),
    createdAt: new Date(),
    projectId,
    description,
    done: false,
  };
}

/** List all admin users. */
export function adminUsers(): UserData[] {
  return users.all().filter((u) => u.role === "admin");
}
