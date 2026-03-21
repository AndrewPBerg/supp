/** Unique identifier type used across all entities. */
export type EntityId = string;

/** Base interface for all storable entities. */
export interface Entity {
  id: EntityId;
  createdAt: Date;
}

/** Supported permission levels. */
export type Role = "viewer" | "editor" | "admin";

/** A user account. */
export interface UserData extends Entity {
  name: string;
  email: string;
  role: Role;
}

/** A project containing tasks. */
export interface ProjectData extends Entity {
  title: string;
  ownerId: EntityId;
  tags: string[];
}

/** A single task within a project. */
export interface TaskData extends Entity {
  projectId: EntityId;
  description: string;
  done: boolean;
}
