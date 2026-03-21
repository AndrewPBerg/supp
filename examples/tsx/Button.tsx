import { useState } from "react";
import { ButtonProps } from "./types";

/** A styled button with click tracking. */
const Button = ({ label, onClick, disabled, variant = "primary" }: ButtonProps) => {
  const [clicks, setClicks] = useState(0);

  const handleClick = () => {
    setClicks(clicks + 1);
    onClick();
  };

  return (
    <button
      className={`btn btn-${variant}`}
      disabled={disabled}
      onClick={handleClick}
    >
      {label} ({clicks})
    </button>
  );
};

export default Button;
