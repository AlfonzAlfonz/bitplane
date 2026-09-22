/**
 * The three implementation-status icons, drawn as one family.
 *
 * Every one is the same circle with a different interior, so the set reads as
 * a single status badge rather than three unrelated glyphs: a tick for done, a
 * half-filled disc for half-done, a slash for nothing there.
 *
 * Stroke and fill are set inline rather than in a stylesheet because the theme
 * paints `.admonitionIcon svg { fill: … }` at the same specificity, and the
 * winner of that tie depends on stylesheet order. `currentColor` inherits the
 * alert's own foreground, so each icon takes its colour from its admonition.
 */
import React from 'react';

const RADIUS = 9.25;

export function IconImplemented(props) {
  return (
    <StatusIcon {...props}>
      <path d="m7.6 12.2 3 3 5.8-6.4" />
    </StatusIcon>
  );
}

export function IconInProgress(props) {
  return (
    <StatusIcon {...props}>
      {/* The right half, filled: half the work is done. */}
      <path
        d={`M12 ${12 - RADIUS}A${RADIUS} ${RADIUS} 0 0 1 12 ${12 + RADIUS}Z`}
        style={{fill: 'currentColor', stroke: 'none'}}
      />
    </StatusIcon>
  );
}

export function IconNotImplemented(props) {
  // The diagonal's ends sit on the circle: 9.25 / √2 either side of centre,
  // rounded so the path data is readable rather than seventeen digits long.
  const offset = Number((RADIUS / Math.SQRT2).toFixed(2));

  return (
    <StatusIcon {...props}>
      <path d={`M${12 - offset} ${12 + offset}L${12 + offset} ${12 - offset}`} />
    </StatusIcon>
  );
}

function StatusIcon({children, ...props}) {
  return (
    <svg
      viewBox="0 0 24 24"
      style={{
        fill: 'none',
        stroke: 'currentColor',
        strokeWidth: 2,
        strokeLinecap: 'round',
        strokeLinejoin: 'round',
      }}
      {...props}>
      <circle cx="12" cy="12" r={RADIUS} />
      {children}
    </svg>
  );
}
