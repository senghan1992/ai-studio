const ALPHABET = 'abcdefghijklmnopqrstuvwxyz0123456789';

/** Short, url-safe, human-typeable id. */
export function shortId(len = 6) {
  let out = '';
  for (let i = 0; i < len; i++) out += ALPHABET[Math.floor(Math.random() * ALPHABET.length)];
  return out;
}

export const newProjectId = () => `prj_${shortId(7)}`;
export const newSlideId = () => `s_${shortId(5)}`;
export const newSectionId = () => `sec_${shortId(5)}`;
export const newSheetId = () => `sh_${shortId(5)}`;
export const newBlockId = () => `b_${shortId(5)}`;

/** Turn a title into a filesystem-safe slug, keeping unicode letters (한글 등). */
export function slugify(title, fallback = 'untitled') {
  const s = String(title ?? '')
    .normalize('NFC')
    .replace(/[/\\?%*:|"<>.]/g, '')
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '')
    .toLowerCase();
  return s || fallback;
}

/** Zero-padded ordinal prefix for stable file sort: 1 -> "01". */
export function pad(n, width = 2) {
  return String(n).padStart(width, '0');
}

/** Ensure `id` is unique within `taken`, suffixing -2, -3 ... */
export function uniqueId(id, taken) {
  if (!taken.has(id)) return id;
  let i = 2;
  while (taken.has(`${id}-${i}`)) i++;
  return `${id}-${i}`;
}
