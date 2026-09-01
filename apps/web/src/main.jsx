import React from 'react';
import { createRoot } from 'react-dom/client';
import App from './App.jsx';
import { initCore } from './core/index.js';
import './styles.css';

/**
 * Load the Rust core before mounting.
 *
 * Everything the editors do to a document — recalculating a sheet, deriving the
 * markdown that will be saved, resolving a chart's range — runs through the wasm
 * module, and they call it synchronously. So it has to be ready before the first
 * render rather than resolved later.
 */
const root = createRoot(document.getElementById('root'));

initCore().then(
  () => {
    root.render(
      <React.StrictMode>
        <App />
      </React.StrictMode>
    );
  },
  (error) => {
    console.error(error);
    root.render(
      <div className="fatal">
        <h1>코어를 불러올 수 없습니다</h1>
        <p>{String(error?.message ?? error)}</p>
        <p className="hint">
          개발 중이라면 <code>npm run build:wasm</code> 을 먼저 실행하세요.
        </p>
      </div>
    );
  }
);
