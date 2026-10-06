/* Semantic icons shared by the details window, panel and editors. */
globalThis.uiIcon = (name) => {
  const paths = {trend:'M3 3v18h18M6 15l5-5 4 3 6-8',chevron:'m6 9 6 6 6-6',right:'m9 6 6 6-6 6',plus:'M12 5v14M5 12h14',minus:'M5 12h14',x:'m6 6 12 12M18 6 6 18',grip:'M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01',up:'m6 14 6-6 6 6',down:'m6 10 6 6 6-6',sort:'M8 4v16m-4-4 4 4 4-4M16 20V4m-4 4 4-4 4 4',refresh:'M20 11a8 8 0 1 0-2.34 5.66M20 5v6h-6',sources:'M3 7h18v13H3ZM7 7V3h10v4',hosts:'M3 3h18v7H3ZM3 14h18v7H3ZM7 6h.01M7 17h.01',prices:'M12 2v20m5-16H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6',wakeups:'M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',general:'M3 7h18M3 17h18M8 4v6M16 14v6',attention:'M12 8v5m0 4h.01M12 3 2 21h20Z'};
  return `<svg class="ui-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="${paths[name] || paths.right}"/></svg>`;
};
