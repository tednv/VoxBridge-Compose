import { render } from 'preact'
import './index.css'
import App from './App.tsx'
import { tokens, tokensToCssVars } from './design-tokens.ts';

// Global Design Token Initialization
const initDesignTokens = () => {
  const cssVars = tokensToCssVars(tokens);
  const root = document.documentElement;
  Object.entries(cssVars).forEach(([key, value]) => {
    root.style.setProperty(key, value as string);
  });
};

initDesignTokens();

const Main = () => {
  return <App />
}

const rootElement = document.getElementById('root');
if (rootElement) {
  render(<Main />, rootElement);
}


