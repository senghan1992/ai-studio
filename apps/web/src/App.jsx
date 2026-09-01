import React, { useCallback, useEffect, useState } from 'react';

import Launcher from './components/Launcher.jsx';
import { Toast, useToast } from './components/ui.jsx';
import { useProject } from './lib/useProject.js';
import DeckEditor from './deck/DeckEditor.jsx';
import DocEditor from './doc/DocEditor.jsx';
import GridEditor from './grid/GridEditor.jsx';

const EDITORS = { deck: DeckEditor, doc: DocEditor, grid: GridEditor };

/** `#/deck/my-file.aideck` — a reloadable location without a router dependency. */
function readHash() {
  const raw = decodeURIComponent(window.location.hash.replace(/^#\/?/, ''));
  if (!raw) return null;
  const slash = raw.indexOf('/');
  if (slash < 0) return null;
  const type = raw.slice(0, slash);
  const folder = raw.slice(slash + 1);
  return type in EDITORS && folder ? { type, folder } : null;
}

export default function App() {
  const [route, setRoute] = useState(readHash);
  const { toast, notify, fail, clear } = useToast();

  useEffect(() => {
    const onHash = () => setRoute(readHash());
    window.addEventListener('hashchange', onHash);
    return () => window.removeEventListener('hashchange', onHash);
  }, []);

  const open = useCallback((folder, type) => {
    window.location.hash = `#/${type}/${encodeURIComponent(folder)}`;
    setRoute({ type, folder });
  }, []);

  const home = useCallback(() => {
    window.location.hash = '';
    setRoute(null);
  }, []);

  return (
    <>
      {route ? (
        <EditorRoute
          key={route.folder}
          route={route}
          onHome={home}
          notify={notify}
          fail={fail}
          onFolderChange={(folder) => open(folder, route.type)}
        />
      ) : (
        <div className="app">
          <Launcher onOpen={open} onError={fail} notify={notify} />
        </div>
      )}
      <Toast message={toast.message} tone={toast.tone} onDone={clear} />
    </>
  );
}

/**
 * Separate component so `useProject` remounts cleanly when the folder changes,
 * rather than trying to reconcile two different documents in one hook instance.
 */
function EditorRoute({ route, onHome, notify, fail, onFolderChange }) {
  const ctl = useProject(route.folder, { onError: fail, onFolderChange });
  const Editor = EDITORS[route.type];

  if (ctl.loading) {
    return (
      <div className="app">
        <div className="center-note">
          <div className="spinner" />
          문서를 불러오는 중…
        </div>
      </div>
    );
  }

  if (!ctl.project) {
    return (
      <div className="app">
        <div className="center-note">
          <p>문서를 열 수 없습니다.</p>
          <button className="btn btn--primary" onClick={onHome}>
            시작 화면으로
          </button>
        </div>
      </div>
    );
  }

  if (ctl.project.type !== route.type) {
    // The hash claimed one type but the folder is another; trust the folder.
    const Actual = EDITORS[ctl.project.type];
    return <Actual ctl={ctl} onHome={onHome} notify={notify} onNewProject={onHome} />;
  }

  return <Editor ctl={ctl} onHome={onHome} notify={notify} onNewProject={onHome} />;
}
