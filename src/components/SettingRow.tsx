import { ComponentChildren } from 'preact';
import { useState } from 'preact/hooks';
import {
  getSettingRowStyle,
  settingRowContentStyle,
  settingRowDescriptionStyle,
  settingRowHeaderStyle,
  settingRowHeaderRightStyle,
  settingRowLabelBadgeStyle,
  settingRowLabelStyle,
  settingRowStatusStyle,
} from '../theme/component-styles.ts';

interface SettingRowProps {
  title: string;
  titleBadge?: string;
  description?: string;
  helpText?: string;
  status?: ComponentChildren;
  children?: ComponentChildren;
  className?: string;
}

export const SettingRow = ({
  title,
  titleBadge,
  description,
  helpText,
  status,
  children,
  className = '',
}: SettingRowProps) => {
  const isReady = className.split(/\s+/).includes('ready');
  const [helpVisible, setHelpVisible] = useState(false);

  return (
    <div
      className={`setting-row ${className}`.trim()}
      style={getSettingRowStyle({ ready: isReady })}
    >
      <div className="setting-row-header" style={settingRowHeaderStyle}>
        <div className="field-label" style={{ ...settingRowLabelStyle, display: 'inline-flex', alignItems: 'center', gap: '7px' }}>
          {title}
          {helpText ? (
            <span style={{ position: 'relative', display: 'inline-flex' }}>
              <button
                type="button"
                aria-label={`${title}: ${helpText}`}
                aria-expanded={helpVisible}
                onMouseEnter={() => setHelpVisible(true)}
                onMouseLeave={() => setHelpVisible(false)}
                onFocus={() => setHelpVisible(true)}
                onBlur={() => setHelpVisible(false)}
                style={{
                  width: '17px',
                  height: '17px',
                  padding: 0,
                  borderRadius: '50%',
                  border: '1px solid rgba(255,255,255,.18)',
                  background: 'transparent',
                  color: 'rgba(231,226,216,.58)',
                  fontSize: '11px',
                  lineHeight: '15px',
                  cursor: 'help',
                }}
              >
                ?
              </button>
              {helpVisible && (
                <span
                  role="tooltip"
                  style={{
                    position: 'absolute',
                    zIndex: 50,
                    top: '23px',
                    left: 0,
                    width: 'min(340px, 45vw)',
                    padding: '9px 11px',
                    borderRadius: '7px',
                    border: '1px solid rgba(255,138,0,.22)',
                    background: 'rgba(18,17,15,.98)',
                    boxShadow: '0 10px 30px rgba(0,0,0,.4)',
                    color: '#d7d1c7',
                    fontSize: '11px',
                    fontWeight: 450,
                    lineHeight: 1.45,
                    whiteSpace: 'normal',
                    pointerEvents: 'none',
                  }}
                >
                  {helpText}
                </span>
              )}
            </span>
          ) : null}
        </div>
        {(titleBadge || status) ? (
          <div className="setting-row-right" style={settingRowHeaderRightStyle}>
            {titleBadge ? <span style={settingRowLabelBadgeStyle}>{titleBadge}</span> : null}
            {status ? <div className="setting-row-status" style={settingRowStatusStyle}>{status}</div> : null}
          </div>
        ) : null}
      </div>
      {description ? <p className="field-description" style={settingRowDescriptionStyle}>{description}</p> : null}
      {children != null ? <div className="field-content" style={settingRowContentStyle}>{children}</div> : null}
    </div>
  );
};
