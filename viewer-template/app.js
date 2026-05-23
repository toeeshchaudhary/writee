// writee static viewer. Vanilla JS, no deps. doc.js exposes window.WRITEE_DOC.
(function () {
  "use strict";

  const canvas = document.getElementById("canvas");
  const ctx = canvas.getContext("2d", { alpha: false });
  const doc = window.WRITEE_DOC || { objects: [] };

  let dpr = window.devicePixelRatio || 1;
  let view = { x: 0, y: 0, zoom: 1 };
  let dragging = null;

  function resize() {
    dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(window.innerWidth * dpr);
    canvas.height = Math.floor(window.innerHeight * dpr);
    draw();
  }
  window.addEventListener("resize", resize);

  // ----- camera ------------------------------------------------------

  function screenToWorld(px, py) {
    return { x: view.x + px / view.zoom, y: view.y + py / view.zoom };
  }

  function fitToContent() {
    const bb = contentBBox();
    if (!bb) {
      view = { x: -window.innerWidth / 2, y: -window.innerHeight / 2, zoom: 1 };
      draw();
      return;
    }
    const pad = 60;
    const w = (bb.maxX - bb.minX) + pad * 2;
    const h = (bb.maxY - bb.minY) + pad * 2;
    const zx = window.innerWidth / w;
    const zy = window.innerHeight / h;
    const zoom = Math.min(zx, zy, 8);
    view.zoom = Math.max(0.05, zoom);
    view.x = bb.minX - pad - (window.innerWidth / view.zoom - w) / 2;
    view.y = bb.minY - pad - (window.innerHeight / view.zoom - h) / 2;
    draw();
  }

  function contentBBox() {
    let any = false;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    for (const o of doc.objects) {
      const bb = objectBBox(o);
      if (!bb) continue;
      any = true;
      if (bb.minX < minX) minX = bb.minX;
      if (bb.minY < minY) minY = bb.minY;
      if (bb.maxX > maxX) maxX = bb.maxX;
      if (bb.maxY > maxY) maxY = bb.maxY;
    }
    return any ? { minX, minY, maxX, maxY } : null;
  }

  function objectBBox(o) {
    if (o.kind === "stroke") {
      if (!o.points || o.points.length === 0) return null;
      let mnx = Infinity, mny = Infinity, mxx = -Infinity, mxy = -Infinity;
      const w = o.width || 4;
      for (const p of o.points) {
        const r = (w * 0.5) * Math.max(0.4, Math.pow(p[2] || 1, 0.7));
        if (p[0] - r < mnx) mnx = p[0] - r;
        if (p[1] - r < mny) mny = p[1] - r;
        if (p[0] + r > mxx) mxx = p[0] + r;
        if (p[1] + r > mxy) mxy = p[1] + r;
      }
      return { minX: mnx, minY: mny, maxX: mxx, maxY: mxy };
    } else if (o.kind === "arrow") {
      const r = (o.head || 14) + (o.width || 1.8);
      return {
        minX: Math.min(o.start[0], o.end[0]) - r,
        minY: Math.min(o.start[1], o.end[1]) - r,
        maxX: Math.max(o.start[0], o.end[0]) + r,
        maxY: Math.max(o.start[1], o.end[1]) + r,
      };
    } else if (o.kind === "text") {
      const lines = (o.content || "").split("\n");
      const maxChars = lines.reduce((m, l) => Math.max(m, l.length), 0);
      const w = Math.max(1, maxChars) * o.size * 0.55;
      const h = Math.max(1, lines.length) * o.size * 1.25;
      return { minX: o.x, minY: o.y, maxX: o.x + w, maxY: o.y + h };
    }
    return null;
  }

  // ----- drawing -----------------------------------------------------

  function drawGrid() {
    const w = canvas.width, h = canvas.height;
    const spacing = 24;
    const dotR = 1.2 * dpr;
    ctx.fillStyle = "#fbfbfb";
    ctx.fillRect(0, 0, w, h);
    ctx.fillStyle = "#9e9e9e";
    const startX = Math.floor(view.x / spacing) * spacing;
    const startY = Math.floor(view.y / spacing) * spacing;
    const endX = view.x + window.innerWidth / view.zoom;
    const endY = view.y + window.innerHeight / view.zoom;
    for (let wx = startX; wx <= endX + spacing; wx += spacing) {
      for (let wy = startY; wy <= endY + spacing; wy += spacing) {
        const sx = (wx - view.x) * view.zoom * dpr;
        const sy = (wy - view.y) * view.zoom * dpr;
        ctx.beginPath();
        ctx.arc(sx, sy, dotR, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  }

  function applyViewTransform() {
    ctx.setTransform(view.zoom * dpr, 0, 0, view.zoom * dpr, -view.x * view.zoom * dpr, -view.y * view.zoom * dpr);
  }

  function smooth(points) {
    // Light moving-average smoothing on (x, y, pressure). Sufficient for a
    // static viewer; the editor side runs a one-Euro filter when capturing.
    if (points.length < 3) return points.map(p => ({ x: p[0], y: p[1], p: p[2] || 1 }));
    const out = [{ x: points[0][0], y: points[0][1], p: points[0][2] || 1 }];
    for (let i = 1; i < points.length - 1; i++) {
      const a = points[i - 1], b = points[i], c = points[i + 1];
      out.push({
        x: (a[0] + 2 * b[0] + c[0]) / 4,
        y: (a[1] + 2 * b[1] + c[1]) / 4,
        p: ((a[2] || 1) + 2 * (b[2] || 1) + (c[2] || 1)) / 4,
      });
    }
    const last = points[points.length - 1];
    out.push({ x: last[0], y: last[1], p: last[2] || 1 });
    return out;
  }

  function resampleCatmullRom(pts, stepsPerSeg) {
    if (pts.length < 2) return pts;
    const out = [];
    for (let i = 0; i < pts.length - 1; i++) {
      const p0 = pts[Math.max(0, i - 1)];
      const p1 = pts[i];
      const p2 = pts[i + 1];
      const p3 = pts[Math.min(pts.length - 1, i + 2)];
      for (let s = 0; s < stepsPerSeg; s++) {
        const t = s / stepsPerSeg;
        out.push({
          x: catmull(p0.x, p1.x, p2.x, p3.x, t),
          y: catmull(p0.y, p1.y, p2.y, p3.y, t),
          p: catmull(p0.p, p1.p, p2.p, p3.p, t),
        });
      }
    }
    out.push(pts[pts.length - 1]);
    return out;
  }

  function catmull(p0, p1, p2, p3, t) {
    const t2 = t * t, t3 = t2 * t;
    return 0.5 * (
      (2 * p1) +
      (-p0 + p2) * t +
      (2 * p0 - 5 * p1 + 4 * p2 - p3) * t2 +
      (-p0 + 3 * p1 - 3 * p2 + p3) * t3
    );
  }

  function strokePolygon(points, widthBase) {
    if (points.length < 2) return null;
    const smoothed = smooth(points);
    const dense = resampleCatmullRom(smoothed, 6);
    const left = [], right = [];
    const n = dense.length;
    for (let i = 0; i < n; i++) {
      const prev = dense[Math.max(0, i - 1)];
      const next = dense[Math.min(n - 1, i + 1)];
      let dx = next.x - prev.x, dy = next.y - prev.y;
      const len = Math.hypot(dx, dy) || 1;
      dx /= len; dy /= len;
      const nx = -dy, ny = dx;
      const half = Math.max(0.35, widthBase * Math.pow(dense[i].p, 0.7) * 0.5);
      left.push({ x: dense[i].x + nx * half, y: dense[i].y + ny * half });
      right.push({ x: dense[i].x - nx * half, y: dense[i].y - ny * half });
    }
    return left.concat(right.reverse());
  }

  function drawStroke(o) {
    const poly = strokePolygon(o.points, o.width || 4);
    if (!poly) return;
    ctx.beginPath();
    ctx.moveTo(poly[0].x, poly[0].y);
    for (let i = 1; i < poly.length; i++) ctx.lineTo(poly[i].x, poly[i].y);
    ctx.closePath();
    ctx.fillStyle = "#111";
    ctx.fill();
  }

  function drawArrow(o) {
    const sx = o.start[0], sy = o.start[1];
    const ex = o.end[0], ey = o.end[1];
    const dx = ex - sx, dy = ey - sy;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len, uy = dy / len;
    const head = o.head || 14;
    const halfHead = head * 0.55;
    const baseX = ex - ux * head * 0.85;
    const baseY = ey - uy * head * 0.85;
    const nx = -uy, ny = ux;
    const w = o.width || 1.8;

    // Shaft (rectangle from start to baseX/baseY of width 2*w).
    ctx.beginPath();
    ctx.moveTo(sx + nx * w, sy + ny * w);
    ctx.lineTo(baseX + nx * w, baseY + ny * w);
    ctx.lineTo(baseX - nx * w, baseY - ny * w);
    ctx.lineTo(sx - nx * w, sy - ny * w);
    ctx.closePath();
    ctx.fillStyle = "#111";
    ctx.fill();
    // Head triangle.
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(baseX + nx * halfHead, baseY + ny * halfHead);
    ctx.lineTo(baseX - nx * halfHead, baseY - ny * halfHead);
    ctx.closePath();
    ctx.fill();
  }

  function drawText(o) {
    // o.size is in world units; the transform already applies the zoom.
    ctx.fillStyle = "#1a1a1a";
    ctx.font = `${o.size}px -apple-system, "Segoe UI", Roboto, sans-serif`;
    ctx.textBaseline = "top";
    const lineH = o.size * 1.25;
    const lines = (o.content || "").split("\n");
    for (let i = 0; i < lines.length; i++) {
      ctx.fillText(lines[i], o.x, o.y + i * lineH);
    }
  }

  function draw() {
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    drawGrid();
    applyViewTransform();
    for (const o of doc.objects) {
      if (o.kind === "stroke") drawStroke(o);
      else if (o.kind === "arrow") drawArrow(o);
      else if (o.kind === "text") drawText(o);
    }
  }

  // ----- input -------------------------------------------------------

  canvas.addEventListener("mousedown", (e) => {
    dragging = { sx: e.clientX, sy: e.clientY, ox: view.x, oy: view.y };
  });
  window.addEventListener("mouseup", () => { dragging = null; });
  window.addEventListener("mousemove", (e) => {
    if (!dragging) return;
    view.x = dragging.ox - (e.clientX - dragging.sx) / view.zoom;
    view.y = dragging.oy - (e.clientY - dragging.sy) / view.zoom;
    draw();
  });

  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const factor = Math.pow(1.15, -e.deltaY / 60);
    const pivot = screenToWorld(e.clientX, e.clientY);
    const newZoom = Math.max(0.05, Math.min(50, view.zoom * factor));
    view.x = pivot.x - (e.clientX) / newZoom;
    view.y = pivot.y - (e.clientY) / newZoom;
    view.zoom = newZoom;
    draw();
  }, { passive: false });

  canvas.addEventListener("dblclick", fitToContent);

  // Touch: one-finger drag pan, two-finger pinch zoom.
  let touches = null;
  canvas.addEventListener("touchstart", (e) => {
    e.preventDefault();
    if (e.touches.length === 1) {
      const t = e.touches[0];
      dragging = { sx: t.clientX, sy: t.clientY, ox: view.x, oy: view.y };
    } else if (e.touches.length === 2) {
      const [a, b] = [e.touches[0], e.touches[1]];
      touches = { dist: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY), zoom: view.zoom };
    }
  }, { passive: false });
  canvas.addEventListener("touchmove", (e) => {
    e.preventDefault();
    if (e.touches.length === 1 && dragging) {
      const t = e.touches[0];
      view.x = dragging.ox - (t.clientX - dragging.sx) / view.zoom;
      view.y = dragging.oy - (t.clientY - dragging.sy) / view.zoom;
      draw();
    } else if (e.touches.length === 2 && touches) {
      const [a, b] = [e.touches[0], e.touches[1]];
      const d = Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
      view.zoom = Math.max(0.05, Math.min(50, touches.zoom * d / touches.dist));
      draw();
    }
  }, { passive: false });
  canvas.addEventListener("touchend", () => { dragging = null; touches = null; });

  resize();
  fitToContent();
})();
