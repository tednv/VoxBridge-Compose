
import { ComponentChildren } from 'preact';
import type { JSX } from 'preact';
import { useState } from 'preact/hooks';
import { tokens } from '../design-tokens.ts';

interface CardProps {
  children: ComponentChildren;
  className?: string;
  variant?: 'primary' | 'secondary';
  onClick?: () => void;
  style?: JSX.CSSProperties;
}

export const Card = ({ children, className = '', variant = 'secondary', onClick, style: styleOverride }: CardProps) => {
  const [hovered, setHovered] = useState(false);

  const style = {
    padding: tokens.spacing.lg,
    borderRadius: tokens.radii.panel,
    background: variant === 'primary' ? 'rgba(29, 28, 25, 0.92)' : 'rgba(18, 18, 16, 0.9)',
    backdropFilter: `blur(${tokens.colors.glassBlur})`,
    border: '1px solid rgba(255, 255, 255, 0.08)',
    boxShadow: tokens.shadows.md,
    transition: tokens.transitions.normal,
    transform: hovered && onClick ? 'translateY(-2px)' : 'translateY(0)',
    cursor: onClick ? 'pointer' : 'default',
  } as const;

  return (
    <div
      className={className}
      onClick={onClick}
      style={{ ...style, ...styleOverride }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      {children}
    </div>
  );
};
