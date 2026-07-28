import type { JSX } from 'preact';
import { tokens } from '../design-tokens.ts';

export type Style = JSX.CSSProperties;

export const titleBarHeight = '42px';

export const appShellStyle: Style = {
  display: 'flex',
  flexDirection: 'column',
  width: '100%',
  height: '100%',
  position: 'relative',
  background: `radial-gradient(circle at 15% 0%, ${tokens.colors.bgGradientWarm} 0%, transparent 36%), linear-gradient(135deg, ${tokens.colors.bgPrimary} 35%, ${tokens.colors.bgGradientCool} 100%)`,
  color: tokens.colors.textPrimary,
};

export const titleBarStyle: Style = {
  height: titleBarHeight,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'space-between',
  padding: '0 10px 0 18px',
  background: 'rgba(9, 9, 8, 0.94)',
  backdropFilter: 'blur(10px)',
  borderBottom: '1px solid rgba(255, 138, 0, 0.12)',
  userSelect: 'none',
  WebkitUserSelect: 'none',
};

export const titleBarTitleStyle: Style = {
  fontSize: '12px',
  fontWeight: 700,
  letterSpacing: '0.04em',
  color: tokens.colors.textPrimary,
};

export const titleBarControlsStyle: Style = {
  display: 'flex',
  alignItems: 'center',
  gap: '6px',
  paddingRight: '2px',
};

export const tabNavStyle: Style = {
  display: 'flex',
  gap: '6px',
  padding: '10px 14px',
  background: 'rgba(12, 12, 11, 0.86)',
  backdropFilter: 'blur(10px)',
  WebkitBackdropFilter: 'blur(10px)',
  borderBottom: '1px solid rgba(255, 255, 255, 0.07)',
  alignItems: 'stretch',
};

export const appContentStyle: Style = {
  flex: 1,
  minHeight: 0,
  overflow: 'auto',
};

export const tabPanelStyle: Style = {
  width: '100%',
  minHeight: '100%',
  padding: '18px',
  display: 'flex',
  flexDirection: 'column',
};

export const tabPanelPaddedStyle: Style = {
  width: '100%',
  maxWidth: '1180px',
  margin: '0 auto',
  display: 'flex',
  flexDirection: 'column',
  gap: '16px',
};

export const tabPanelContentStyle: Style = {
  width: '100%',
  maxWidth: '1180px',
  margin: '0 auto',
  display: 'flex',
  flexDirection: 'column',
};

export const inputBaseStyle: Style = {
  width: '100%',
  background: 'rgba(255, 255, 255, 0.035)',
  color: tokens.colors.textPrimary,
  border: '1px solid rgba(255, 255, 255, 0.11)',
  borderRadius: tokens.radii.input,
  padding: '10px 12px',
  fontSize: tokens.typography.sizeSm,
  outline: 'none',
};

export const selectWrapperStyle: Style = {
  display: 'flex',
  gap: tokens.spacing.sm,
  width: '100%',
  alignItems: 'center',
};

export const helperTextStyle: Style = {
  fontSize: tokens.typography.sizeXs,
  color: '#d9dfe7',
  lineHeight: 1.4,
};

export const toastContainerStyle: Style = {
  position: 'fixed',
  top: '50%',
  left: '50%',
  transform: 'translate(-50%, -50%)',
  display: 'flex',
  flexDirection: 'column',
  alignItems: 'center',
  gap: '8px',
  zIndex: 1000,
  width: 'min(420px, calc(100% - 24px))',
  padding: '0 12px',
  boxSizing: 'border-box',
  pointerEvents: 'none',
};

export const getToastStyle = (type: 'success' | 'error' | 'info' | 'private' | 'saved'): Style => ({
  width: type === 'saved' ? 'auto' : '100%',
  maxWidth: type === 'saved' ? '220px' : '100%',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  padding: type === 'saved' ? '2px 4px' : '10px 12px',
  borderRadius: type === 'saved' ? '4px' : '10px',
  border: 'none',
  background: type === 'saved'
    ? 'transparent'
    : type === 'success'
      ? '#10b981'
      : type === 'error'
        ? '#ef4444'
        : type === 'private'
          ? tokens.colors.privacy
          : '#4cc9f0',
  cursor: type === 'saved' ? 'default' : 'pointer',
  pointerEvents: 'auto',
  boxShadow: type === 'saved' ? 'none' : '0 4px 12px rgba(0, 0, 0, 0.22)',
});

export const toastDotStyle: Style = {
  width: '8px',
  height: '8px',
  borderRadius: '999px',
  background: tokens.colors.accentPrimary,
  flexShrink: 0,
};

export const toastMessageStyle: Style = {
  fontSize: tokens.typography.sizeSm,
  color: tokens.colors.textPrimary,
};

export const getToastMessageStyle = (type: 'success' | 'error' | 'info' | 'private' | 'saved'): Style => ({
  fontSize: type === 'saved' ? tokens.typography.sizeXs : tokens.typography.sizeSm,
  color: type === 'saved' ? tokens.colors.textMuted : tokens.colors.textPrimary,
  fontWeight: type === 'saved' ? 500 : 500,
  letterSpacing: type === 'saved' ? '0.01em' : 'normal',
});

export const modalTextIntroStyle: Style = {
  ...helperTextStyle,
  marginBottom: '10px',
};

export const modalShortcutPathStyle: Style = {
  fontSize: tokens.typography.sizeSm,
  color: tokens.colors.textPrimary,
  fontWeight: 600,
  marginBottom: '8px',
};

export const modalShortcutNoteStyle: Style = {
  ...helperTextStyle,
};
