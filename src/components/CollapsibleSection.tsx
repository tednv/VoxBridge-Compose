
import { ComponentChildren } from 'preact';
import { tokens } from '../design-tokens.ts';

interface CollapsibleSectionProps {
  title: string;
  children: ComponentChildren;
  isOpen: boolean;
  onToggle: () => void;
}

export const CollapsibleSection = ({ children, isOpen }: CollapsibleSectionProps) => {
  if (!isOpen) return null;
  return (
    <section style={{ display: 'flex', flexDirection: 'column', gap: tokens.spacing.sm, padding: `0 ${tokens.spacing.md} ${tokens.spacing.xl}` }}>
      {children}
    </section>
  );
};
