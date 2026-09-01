import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api.js';

const ITEMS_KEY = { deck: 'slides', doc: 'sections', grid: 'sheets' };
const HISTORY_LIMIT = 80;
const AUTOSAVE_MS = 2500;

/**
 * Load, edit, undo and save one project.
 *
 * Edits are applied to a local copy and pushed onto an undo stack; a debounced
 * autosave writes the whole project back, which is what regenerates the md/json
 * pairs and `AI.md` on disk. Coalescing key (`mergeKey`) lets a burst of edits to
 * the same thing — dragging a block, typing in a cell — collapse into one undo
 * step instead of one per mouse move.
 */
export function useProject(folder, { onError, onSaved, onFolderChange } = {}) {
  const [project, setProject] = useState(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [savedAt, setSavedAt] = useState(null);

  const past = useRef([]);
  const future = useRef([]);
  const lastMerge = useRef({ key: null, at: 0 });
  const timer = useRef(null);
  const latest = useRef(null);
  const [historyTick, setHistoryTick] = useState(0);

  const notifyError = useCallback((e) => onError?.(e instanceof Error ? e.message : String(e)), [onError]);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    past.current = [];
    future.current = [];
    api
      .getProject(folder)
      .then((data) => {
        if (!alive) return;
        setProject(data);
        latest.current = data;
        setDirty(false);
        setSavedAt(data.manifest?.modified ?? null);
      })
      .catch((e) => alive && notifyError(e))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [folder, notifyError]);

  const itemsKey = project ? ITEMS_KEY[project.type] : null;

  /** Snapshot only the mutable parts; the folder and format never change here. */
  const snapshot = (p) => ({ manifest: p.manifest, items: p[ITEMS_KEY[p.type]] });

  const commit = useCallback(
    (updater, { mergeKey = null } = {}) => {
      setProject((current) => {
        if (!current) return current;
        const key = ITEMS_KEY[current.type];
        const next = typeof updater === 'function' ? updater(current) : updater;
        if (!next || next === current) return current;

        const now = Date.now();
        const canMerge =
          mergeKey !== null &&
          lastMerge.current.key === mergeKey &&
          now - lastMerge.current.at < 1200;

        if (!canMerge) {
          past.current.push(snapshot(current));
          if (past.current.length > HISTORY_LIMIT) past.current.shift();
          future.current = [];
        }
        lastMerge.current = { key: mergeKey, at: now };

        const merged = { ...current, ...next, [key]: next[key] ?? current[key] };
        latest.current = merged;
        setDirty(true);
        setHistoryTick((t) => t + 1);
        return merged;
      });
    },
    []
  );

  const applySnapshot = useCallback((snap) => {
    setProject((current) => {
      if (!current || !snap) return current;
      const merged = { ...current, manifest: snap.manifest, [ITEMS_KEY[current.type]]: snap.items };
      latest.current = merged;
      return merged;
    });
    setDirty(true);
    setHistoryTick((t) => t + 1);
  }, []);

  const undo = useCallback(() => {
    if (!past.current.length) return;
    const current = latest.current;
    const snap = past.current.pop();
    if (current) future.current.push(snapshot(current));
    lastMerge.current = { key: null, at: 0 };
    applySnapshot(snap);
  }, [applySnapshot]);

  const redo = useCallback(() => {
    if (!future.current.length) return;
    const current = latest.current;
    const snap = future.current.pop();
    if (current) past.current.push(snapshot(current));
    lastMerge.current = { key: null, at: 0 };
    applySnapshot(snap);
  }, [applySnapshot]);

  const save = useCallback(async () => {
    const current = latest.current;
    if (!current) return null;
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    setSaving(true);
    try {
      const key = ITEMS_KEY[current.type];
      const saved = await api.saveProject(folder, {
        manifest: { title: current.manifest?.title, theme: current.manifest?.theme },
        [key]: current[key],
      });
      // Keep the client's items: the server echoes normalised copies, and
      // replacing them mid-edit would fight the user's cursor.
      setProject((p) => (p ? { ...p, manifest: saved.manifest } : p));
      latest.current = { ...current, manifest: saved.manifest };
      setDirty(false);
      setSavedAt(saved.manifest?.modified ?? new Date().toISOString());
      onSaved?.(saved);
      return saved;
    } catch (e) {
      notifyError(e);
      return null;
    } finally {
      setSaving(false);
    }
  }, [folder, notifyError, onSaved]);

  // Debounced autosave. Every commit restarts the clock, so a long editing burst
  // writes once when the user pauses.
  useEffect(() => {
    if (!dirty) return undefined;
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      timer.current = null;
      save();
    }, AUTOSAVE_MS);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [dirty, historyTick, save]);

  // Ctrl/Cmd+S, Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y
  useEffect(() => {
    const onKey = (e) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === 's') {
        e.preventDefault();
        save();
      } else if (key === 'z' && !e.shiftKey) {
        e.preventDefault();
        undo();
      } else if ((key === 'z' && e.shiftKey) || key === 'y') {
        e.preventDefault();
        redo();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [save, undo, redo]);

  // Warn on close with unsaved work.
  useEffect(() => {
    if (!dirty) return undefined;
    const warn = (e) => {
      e.preventDefault();
      e.returnValue = '';
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  }, [dirty]);

  /**
   * Commit a title edit: flush pending content first, then rename the folder so
   * the file name on disk keeps matching the document title. The folder is the
   * project's identity, so the caller is told when it changes.
   */
  const commitTitle = useCallback(
    async (title) => {
      const next = String(title ?? '').trim();
      const current = latest.current?.manifest?.title ?? '';
      if (!next || next === current) return null;
      try {
        if (dirty) await save();
        const saved = await api.renameProject(folder, next);
        if (saved?.folder && saved.folder !== folder) onFolderChange?.(saved.folder);
        else if (saved?.manifest) {
          setProject((p) => (p ? { ...p, manifest: saved.manifest } : p));
          setSavedAt(saved.manifest.modified ?? null);
          setDirty(false);
        }
        return saved;
      } catch (e) {
        notifyError(e);
        return null;
      }
    },
    [folder, dirty, save, notifyError, onFolderChange]
  );

  const setTitle = useCallback(
    (title) => commit((p) => ({ ...p, manifest: { ...p.manifest, title } }), { mergeKey: 'title' }),
    [commit]
  );

  const items = project && itemsKey ? project[itemsKey] : [];
  const setItems = useCallback(
    (updater, options) =>
      commit((p) => {
        const key = ITEMS_KEY[p.type];
        const nextItems = typeof updater === 'function' ? updater(p[key]) : updater;
        return { ...p, [key]: nextItems };
      }, options),
    [commit]
  );

  return useMemo(
    () => ({
      project,
      items,
      itemsKey,
      loading,
      saving,
      dirty,
      savedAt,
      commit,
      setItems,
      setTitle,
      save,
      commitTitle,
      undo,
      redo,
      canUndo: past.current.length > 0,
      canRedo: future.current.length > 0,
    }),
    [project, items, itemsKey, loading, saving, dirty, savedAt, commit, setItems, setTitle, save, commitTitle, undo, redo, historyTick]
  );
}
