import { UserCardProps } from "./types";
import Button from "./Button";

/** Display a user's info with an edit action. */
export function UserCard({ name, email, role, onEdit }: UserCardProps) {
  return (
    <div className="user-card">
      <h3>{name}</h3>
      <p>{email}</p>
      <span className={`badge badge-${role}`}>{role}</span>
      {onEdit && <Button label="Edit" onClick={onEdit} />}
    </div>
  );
}
