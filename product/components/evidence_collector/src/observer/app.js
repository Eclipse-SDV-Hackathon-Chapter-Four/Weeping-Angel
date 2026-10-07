// Live Scenario Observer frontend (ADR-016).
// Consumes the collector's SSE stream: one `snapshot`, then `sample` and
// `detection` deltas. Renders a 20 s sliding window on the relative-time axis.
'use strict';

const WINDOW_FALLBACK = 20000;

let state = {
  run_id: null,
  t0_ms: 0,
  window_ms: WINDOW_FALLBACK,
  bands: null,
  ground_truth: { incidents: [] },
  oracle: null,
  samples: [],
  detections: [],
};

const canvas = document.getElementById('plot');
const ctx = canvas.getContext('2d');
const statusEl = document.getElementById('status');
const runEl = document.getElementById('run');

let dirty = false;
function schedule() {
  if (dirty) return;
  dirty = true;
  requestAnimationFrame(() => { dirty = false; draw(); });
}

function oracleGoals() {
  // Map injection_id -> primary class list, tolerating object or list oracles.
  const out = {};
  const entries = Array.isArray(state.oracle) ? state.oracle
    : state.oracle && typeof state.oracle === 'object' ? [state.oracle] : [];
  for (const o of entries) {
    const goals = (o.generation_goal && o.generation_goal.primary) || o.primary || [];
    out[o.injection_id] = goals.map(g => g.class).filter(Boolean);
  }
  return out;
}

function trim() {
  const last = state.samples.length ? state.samples[state.samples.length - 1].timestamp_ms : 0;
  const win = state.window_ms || WINDOW_FALLBACK;
  const cutoff = Math.max(0, last - win);
  while (state.samples.length && state.samples[0].timestamp_ms < cutoff) state.samples.shift();
  const detCutoff = cutoff;
  state.detections = state.detections.filter(d => d.at_ms >= detCutoff);
}

function applySnapshot(snapshot) {
  state = snapshot;
  state.samples = state.samples || [];
  state.detections = state.detections || [];
  runEl.textContent = state.run_id ? `run ${state.run_id}` : '';
  schedule();
}

// Exported documents embed the final snapshot (ADR-018) and must not open an
// SSE connection; the live page falls through to the stream below.
if (window.__OBSERVER_SNAPSHOT__) {
  statusEl.textContent = 'frozen';
  statusEl.className = 'status live';
  applySnapshot(window.__OBSERVER_SNAPSHOT__);
} else {
  const es = new EventSource('/events');
  es.addEventListener('open', () => { statusEl.textContent = 'live'; statusEl.className = 'status live'; });
  es.addEventListener('error', () => { statusEl.textContent = 'disconnected'; statusEl.className = 'status down'; });
  es.addEventListener('snapshot', e => applySnapshot(JSON.parse(e.data)));
  es.addEventListener('sample', e => {
    state.samples.push(JSON.parse(e.data));
    trim();
    schedule();
  });
  es.addEventListener('detection', e => {
    state.detections.push(JSON.parse(e.data));
    schedule();
  });
}

const LEVEL_COLORS = { WARNING: '#e5c07b', VIOLATION: '#e06c75', CRITICAL: '#c678dd' };

function levelColor(level) {
  return LEVEL_COLORS[(level || '').toUpperCase()] || LEVEL_COLORS.WARNING;
}

function draw() {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth, h = canvas.clientHeight;
  canvas.width = w * dpr; canvas.height = h * dpr;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);

  const m = { l: 56, r: 12, t: 18, b: 26 };
  const pw = Math.max(10, w - m.l - m.r);
  const ph = Math.max(10, h - m.t - m.b);
  const socH = ph * 0.3;
  const gap = 18;
  const tempH = ph - socH - gap;
  const tempTop = m.t;
  const socTop = m.t + tempH + gap;

  const win = state.window_ms || WINDOW_FALLBACK;
  const samples = state.samples;
  const last = samples.length ? samples[samples.length - 1].timestamp_ms : 0;
  const tEnd = Math.max(win, last);
  const tStart = tEnd - win;
  const X = t => m.l + ((t - tStart) / win) * pw;

  const bands = state.bands;
  const absMin = bands ? bands.temp_abs_min_c : -30;
  const absMax = bands ? bands.temp_abs_max_c : 70;
  const warn = bands ? bands.temp_warning_c : absMax - 10;
  const yMin = absMin - 5, yMax = absMax + 5;
  const Yt = v => tempTop + tempH - ((v - yMin) / (yMax - yMin)) * tempH;
  const Ys = v => socTop + socH - (v / 100) * socH;

  // Temperature bands.
  ctx.fillStyle = 'rgba(229,192,123,0.13)';
  ctx.fillRect(m.l, Yt(absMax), pw, Yt(warn) - Yt(absMax));
  ctx.fillStyle = 'rgba(224,108,117,0.18)';
  ctx.fillRect(m.l, Yt(yMax), pw, Yt(absMax) - Yt(yMax));

  // Incident windows across both lanes.
  const goals = oracleGoals();
  for (const inc of (state.ground_truth.incidents || [])) {
    const x0 = X(inc.start_ms), x1 = X(inc.end_ms);
    ctx.fillStyle = 'rgba(97,175,239,0.12)';
    ctx.fillRect(x0, tempTop, Math.max(1, x1 - x0), tempH + gap + socH);
    ctx.strokeStyle = 'rgba(97,175,239,0.5)';
    ctx.strokeRect(x0, tempTop, Math.max(1, x1 - x0), tempH + gap + socH);
    const expected = goals[inc.injection_id] || [];
    const label = inc.injected_class + (expected.length ? ' → ' + expected.join(',') : '');
    ctx.fillStyle = '#61afef';
    ctx.font = '11px system-ui, sans-serif';
    ctx.fillText(label, Math.max(m.l, x0) + 2, tempTop - 5);
  }

  // Time grid.
  ctx.strokeStyle = '#2a2f38';
  ctx.fillStyle = '#8b93a1';
  ctx.font = '10px system-ui, sans-serif';
  ctx.lineWidth = 1;
  const step = 2000;
  for (let t = Math.ceil(tStart / step) * step; t <= tEnd; t += step) {
    const x = X(t);
    ctx.beginPath(); ctx.moveTo(x, tempTop); ctx.lineTo(x, tempTop + tempH);
    ctx.moveTo(x, socTop); ctx.lineTo(x, socTop + socH); ctx.stroke();
    ctx.fillText(`${(t / 1000).toFixed(0)}s`, x + 2, h - 8);
  }

  // Signal lines.
  const line = (Y, key, color) => {
    ctx.strokeStyle = color; ctx.lineWidth = 1.5; ctx.beginPath();
    let started = false;
    for (const s of samples) {
      const v = s[key];
      if (v === null || v === undefined) continue;
      const x = X(s.timestamp_ms), y = Y(v);
      if (!started) { ctx.moveTo(x, y); started = true; } else ctx.lineTo(x, y);
    }
    ctx.stroke();
  };
  line(Yt, 'temp_min', '#61afef');
  line(Yt, 'temp_avg', '#98c379');
  line(Yt, 'temp_max', '#e5c07b');
  line(Ys, 'soc', '#56b6c2');

  // Detection markers (step events on the relative axis).
  ctx.font = '10px system-ui, sans-serif';
  for (const d of state.detections) {
    const x = X(d.at_ms);
    if (x < m.l || x > m.l + pw) continue;
    ctx.strokeStyle = levelColor(d.level);
    ctx.lineWidth = d.stage === 'Passed' ? 1 : 1.5;
    ctx.setLineDash(d.stage === 'Passed' ? [3, 3] : []);
    ctx.beginPath();
    ctx.moveTo(x, tempTop); ctx.lineTo(x, socTop + socH);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  // Axis labels.
  ctx.fillStyle = '#8b93a1';
  ctx.fillText('\u00b0C', 6, tempTop + 10);
  ctx.fillText(`${absMin}`.replace('-', '\u2212'), 6, Yt(absMin));
  ctx.fillText(`${warn}`.replace('-', '\u2212'), 6, Yt(warn) + 3);
  ctx.fillText(`${absMax}`, 6, Yt(absMax) + 3);
  ctx.fillText('%', 6, socTop + 10);
  ctx.fillText('0', 6, socTop + socH);
  ctx.fillText('100', 2, socTop + 9);

  if (!samples.length) {
    ctx.fillStyle = '#8b93a1';
    ctx.fillText('waiting for battery samples…', m.l + 8, tempTop + 20);
  }
}

// Legend: makes the injected ground truth vs. the observed Guardian
// detections explicit, and explains the detection line style (ADR-016).
const legendGroups = [
  {
    title: 'Signals',
    items: [
      { cls: 'line', color: '#61afef', label: 'temp_min' },
      { cls: 'line', color: '#98c379', label: 'temp_avg' },
      { cls: 'line', color: '#e5c07b', label: 'temp_max' },
      { cls: 'line', color: '#56b6c2', label: 'SoC' },
    ],
  },
  {
    title: 'Injected (ground truth)',
    items: [
      { cls: 'window', color: '#61afef', label: 'incident window' },
    ],
  },
  {
    title: 'Detected by Guardian',
    items: [
      { cls: 'vline', color: LEVEL_COLORS.WARNING, label: 'WARNING' },
      { cls: 'vline', color: LEVEL_COLORS.VIOLATION, label: 'VIOLATION' },
      { cls: 'vline', color: LEVEL_COLORS.CRITICAL, label: 'CRITICAL' },
    ],
  },
  {
    title: 'Detection stage',
    items: [
      { cls: 'vline', color: '#8b93a1', label: 'solid = Failed (active)' },
      { cls: 'vline dashed', color: '#8b93a1', label: 'dashed = Passed (cleared)' },
    ],
  },
];
const legend = document.getElementById('legend');
legend.innerHTML = legendGroups.map(g => [
  `<span class="legend-group"><span class="legend-title">${g.title}</span>`,
  ...g.items.map(it => `<span class="legend-item"><span class="swatch ${it.cls}" style="--c:${it.color}"></span>${it.label}</span>`),
  '</span>',
].join('')).join('');

window.addEventListener('resize', schedule);
draw();
