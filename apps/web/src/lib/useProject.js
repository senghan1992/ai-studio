import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api } from '../api.js';

const ITEMS_KEY = { deck: 'slides', doc: 'sections', grid: 'sheets' };
const HISTORY_LIMIT = 80;
const AUTOSAVE_MS = 2500;
/** How often a clean document checks whether someone else saved it. */
const EXTERNAL_POLL_MS = 7000;

/** A refused-overwrite error: HTTP 409, or the same message over Tauri IPC
 *  where no status code travels. */
const isConflict = (e) =>
  e?.status === 409 || String(e?.message ?? e).includes('다른 곳에서 문서가 수정되었습니다');

/**
 * Load, edit, undo and save one project.
 *
 * Edits are applied to a local copy and pushed onto an undo stack; a debounced
 * autosave writes the whole project back, which is what regenerates the md/json
 * pairs and `AI.md` on disk. Coalescing key (`mergeKey`) lets a burst of edits to
 * the same thing — dragging a block, typing in a cell — collapse into one undo
 * step instead of one per mouse move.
 */
export function useProject(folder, { onError, onSaved, onFolderChange, onExternalChange } = {}) {
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
  /** The `manifest.modified` this editor loaded or last wrote. Sent with every
   *  save so the server can refuse to overwrite work done elsewhere. */
  const baseModified = useRef(null);
  /** True after the user declined to overwrite: autosave holds until the next
   *  explicit save, instead of re-asking every 2.5 seconds. */
  const conflictHold = useRef(false);

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
        baseModified.current = data.manifest?.modified ?? null;
        conflictHold.current = false;
        setDirty(false);
        setSavedAt(data.manifest?.modified ?? null);
        // A broken layout/meta/cells file was replaced by defaults on load;
        // saying so is what stands between resilience and silent loss.
        if (data.warnings?.length) onError?.(data.warnings.join('\n'));
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

  const save = useCallback(async (options) => {
    // Callers pass this straight to onClick, so the argument may be an event;
    // only an explicit options object carries flags.
    const force = options?.force === true;
    const auto = options?.auto === true;
    const current = latest.current;
    if (!current) return null;
    // After a declined overwrite, autosave waits for an explicit save instead
    // of re-raising the same conflict every couple of seconds.
    if (auto && conflictHold.current) return null;
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    setSaving(true);
    try {
      const key = ITEMS_KEY[current.type];
      const saved = await api.saveProject(folder, {
        manifest: { title: current.manifest?.title, theme: current.manifest?.theme },
        // The timestamp this editor loaded: the server refuses the save if the
        // disk moved past it, so two writers cannot silently eat each other.
        ...(force ? {} : { baseModified: baseModified.current ?? undefined }),
        [key]: current[key],
      });
      // Keep the client's items: the server echoes normalised copies, and
      // replacing them mid-edit would fight the user's cursor.
      setProject((p) => (p ? { ...p, manifest: saved.manifest } : p));
      latest.current = { ...current, manifest: saved.manifest };
      baseModified.current = saved.manifest?.modified ?? null;
      conflictHold.current = false;
      setDirty(false);
      setSavedAt(saved.manifest?.modified ?? new Date().toISOString());
      onSaved?.(saved);
      return saved;
    } catch (e) {
      if (isConflict(e)) {
        // The same dialog Word shows: someone else saved first. The person at
        // the screen decides which copy wins.
        const overwrite =
          typeof window !== 'undefined' &&
          typeof window.confirm === 'function' &&
          window.confirm(
            '다른 곳에서 이 문서가 수정되었습니다 (다른 탭, 데스크톱 앱, 또는 에이전트).\n\n' +
              '확인 — 지금 화면의 내용으로 덮어씁니다\n' +
              '취소 — 저장을 보류합니다 (새로고침하면 다른 쪽의 내용을 봅니다)'
          );
        if (overwrite) return save({ force: true });
        conflictHold.current = true;
        notifyError('저장을 보류했습니다 — 다시 저장하려면 Ctrl+S, 다른 쪽 내용을 보려면 새로고침하세요.');
        return null;
      }
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
      save({ auto: true });
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
   * Follow saves made elsewhere — an agent over the API, another tab, the
   * desktop app — while this editor is clean.
   *
   * A cheap poll of the project listing compares the disk's save time against
   * the one this editor knows; on a mismatch the document is reloaded in place
   * and the user is told. While the user is editing, nothing polls: the save
   * conflict check (`baseModified`) already guards that path, and reloading
   * under a cursor would be worse than either.
   */
  // The callback lives in a ref so a caller passing an inline closure does not
  // restart the poll timer on every render.
  const externalChange = useRef(onExternalChange);
  externalChange.current = onExternalChange;

  useEffect(() => {
    if (dirty || saving || loading) return undefined;
    let alive = true;
    const check = async () => {
      if (!alive || document.hidden) return;
      try {
        const before = latest.current;
        const list = await api.listProjects();
        const mine = list?.projects?.find((p) => p.folder === folder);
        if (!alive || !mine?.modified || mine.modified === baseModified.current) return;
        const fresh = await api.getProject(folder);
        // A keystroke may have landed while the fetch was in flight — the
        // editor's copy wins, and the conflict check sorts it out at save
        // time. `latest` having moved since the check began is that signal.
        if (!alive || !fresh || latest.current !== before) return;
        setProject(fresh);
        latest.current = fresh;
        baseModified.current = fresh.manifest?.modified ?? null;
        setSavedAt(fresh.manifest?.modified ?? null);
        // The undo trail belongs to the replaced document.
        past.current = [];
        future.current = [];
        setHistoryTick((t) => t + 1);
        externalChange.current?.();
      } catch {
        /* offline or mid-restart: try again next tick */
      }
    };
    const timer = setInterval(check, EXTERNAL_POLL_MS);
    // Coming back to the tab checks immediately: the user is looking now.
    const onVisible = () => {
      if (!document.hidden) check();
    };
    document.addEventListener('visibilitychange', onVisible);
    return () => {
      alive = false;
      clearInterval(timer);
      document.removeEventListener('visibilitychange', onVisible);
    };
  }, [folder, dirty, saving, loading]);

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
          baseModified.current = saved.manifest.modified ?? null;
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
