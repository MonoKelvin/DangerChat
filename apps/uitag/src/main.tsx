import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import { applyTheme, getTheme } from './lib/theme';
import './styles/global.css';

// 渲染前应用持久化主题（默认深色），避免首帧闪烁
applyTheme(getTheme());

// 禁用 WebView2 默认右键菜单（画布上的右键由自定义「改标签」菜单接管）
document.addEventListener('contextmenu', (e) => e.preventDefault());

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
