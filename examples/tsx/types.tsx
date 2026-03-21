/** Props for the Button component. */
export interface ButtonProps {
  label: string;
  onClick: () => void;
  disabled?: boolean;
  variant?: "primary" | "secondary";
}

/** Props for the UserCard component. */
export interface UserCardProps {
  name: string;
  email: string;
  role: "viewer" | "editor" | "admin";
  onEdit?: () => void;
}

/** Props for the App shell. */
export interface AppProps {
  title: string;
  users: UserCardProps[];
}

/** Shared theme tokens. */
export type Theme = {
  primary: string;
  secondary: string;
  fontSize: number;
};
