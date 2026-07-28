
export const tokens = {
  colors: {
    // Backgrounds
    bgPrimary: '#090908',
    bgSecondary: '#111110',
    bgTertiary: '#171715',
    bgHover: '#24221f',
    bgGradientWarm: '#1c1006',
    bgGradientCool: '#0d0d0c',
    
    // Text
    textPrimary: '#f6f1e7',
    textSecondary: '#c8c3ba',
    textMuted: '#87837d',
    
    // Brand/Action
    accentPrimary: '#ff8a00',
    accentHover: '#ffb800',
    accentSoft: '#ffd966',
    privacy: '#9b7cff',
    success: '#4fc38b',
    error: '#ef665e',
    glassBg: 'rgba(20, 19, 17, 0.88)',
    glassBgHeavy: 'rgba(14, 14, 13, 0.97)',
    glassBlur: '12px',
  },
  
  spacing: {
    xs: '4px',
    sm: '8px',
    md: '16px',
    lg: '24px',
    xl: '32px',
  },
  
  radii: {
    input: '7px',
    panel: '14px',
    button: '7px',
  },
  
  shadows: {
    sm: '0 2px 8px rgba(0, 0, 0, 0.2)',
    md: '0 6px 16px rgba(0, 0, 0, 0.3)',
    lg: '0 12px 32px rgba(0, 0, 0, 0.4)',
    accent: '0 8px 24px rgba(255, 138, 0, 0.18)',
  },
  
  transitions: {
    fast: 'all 0.15s cubic-bezier(0.4, 0, 0.2, 1)',
    normal: 'all 0.25s cubic-bezier(0.4, 0, 0.2, 1)',
    slow: 'all 0.4s cubic-bezier(0.4, 0, 0.2, 1)',
  },
  
  typography: {
    fontMain: "'Space Grotesk', 'Segoe UI', system-ui, -apple-system, sans-serif",
    fontMono: "'JetBrains Mono', 'Cascadia Code', 'Fira Code', monospace",
    sizeXs: '11px',

    sizeSm: '13px',
    sizeMd: '14px',
    sizeLg: '16px',
    sizeXl: '18px',
    sizeHuge: '32px',
  }
} as const;

export type DesignTokens = typeof tokens;

// Helper to convert camelCase to kebab-case for CSS variables
export const tokensToCssVars = (obj: any, prefix = '--'): Record<string, string> => {
  const vars: Record<string, string> = {};
  
  const iterate = (current: any, currentPrefix: string) => {
    for (const key in current) {
      const value = current[key];
      const kebabKey = key.replace(/([a-z0-9])([A-Z])/g, '$1-$2').toLowerCase();
      const newPrefix = `${currentPrefix}${kebabKey}`;
      
      if (typeof value === 'object' && value !== null) {
        iterate(value, `${newPrefix}-`);
      } else {
        vars[newPrefix] = value;
      }
    }
  };
  
  iterate(obj, prefix);
  return vars;
};
