/* Semantic icons shared by the details window, panel and editors. */
globalThis.uiIcon = (name) => {
  const paths = {stack:'M4 9h16v12H4ZM6 5h12M8 2h8',switch:'M4 7h16m-4-4 4 4-4 4M20 17H4m4-4-4 4 4 4',credit:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M15 9a4 4 0 1 0 0 6',terminal:'M3 4h18v16H3ZM7 8l3 4-3 4M13 16h4',trend:'M3 3v18h18M6 15l5-5 4 3 6-8',chevron:'m6 9 6 6 6-6',back:'m15 18-6-6 6-6',right:'m9 6 6 6-6 6',plus:'M12 5v14M5 12h14',minus:'M5 12h14',minusCircle:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M8 12h8',x:'m6 6 12 12M18 6 6 18',grip:'M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01',up:'m6 14 6-6 6 6',down:'m6 10 6 6 6-6',sort:'M8 4v16m-4-4 4 4 4-4M16 20V4m-4 4 4-4 4 4',refresh:'M20 11a8 8 0 1 0-2.34 5.66M20 5v6h-6',sources:'M3 7h18v13H3ZM7 7V3h10v4',hosts:'M3 3h18v7H3ZM3 14h18v7H3ZM7 6h.01M7 17h.01',prices:'M12 2v20m5-16H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6',wakeups:'M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',general:'M3 7h18M3 17h18M8 4v6M16 14v6',attention:'M12 8v5m0 4h.01M12 3 2 21h20Z'};
  Object.assign(paths, {
    // Navigation follows the macOS sidebar symbols: square.grid.2x2, person.crop.circle,
    // server.rack (hosts) and desktopcomputer.
    overview:'M3 3h7v7H3ZM14 3h7v7h-7ZM3 14h7v7H3ZM14 14h7v7h-7Z',
    account:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M15 10a3 3 0 1 1-6 0 3 3 0 0 1 6 0M6.2 18.4a7 7 0 0 1 11.6 0',
    computer:'M3 4h18v12H3ZM8 21h8M12 16v5',
    sidebarOpen:'M3 4h18v16H3ZM9 4v16m4-12 4 4-4 4',
    sidebarClose:'M3 4h18v16H3ZM9 4v16m8-12-4 4 4 4',
    pet:'M8 14c-1 2-3 2-3 4s2 3 4 2c2-1 4-1 6 0s4 0 4-2-2-2-3-4c-2-3-6-3-8 0M6 5c-3 0-3 5 0 5S9 5 6 5M11 3c-3 0-3 6 0 6s3-6 0-6M16 4c-3 0-3 5 0 5s3-5 0-5M20 8c-2 0-2 5 0 5s2-5 0-5',
    network:'M12 9v7M5 16h14M5 16v3m14-3v3M8 3h8v6H8ZM2 19h6v3H2Zm14 0h6v3h-6Z',
    sun:'M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5',
    moon:'M20.5 14a8.5 8.5 0 0 1-10.5-10.5A8.5 8.5 0 1 0 20.5 14Z',themeSystem:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 3v18m0-13.5h6m-6 4.5h8.5m-8.5 4.5h6',
    download:'M12 3v12m-5-5 5 5 5-5M5 16v5h14v-5',
    expand:'M14 4h6v6M20 4l-8 8M10 20H4v-6',
    settings:'m9.7 3-.5 2.2-1.7 1-2.2-.7-2.3 4 1.7 1.5v2L3 14.5l2.3 4 2.2-.7 1.7 1 .5 2.2h4.6l.5-2.2 1.7-1 2.2.7 2.3-4-1.7-1.5v-2L21 9.5l-2.3-4-2.2.7-1.7-1-.5-2.2ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    more:'M5 12h.01M12 12h.01M19 12h.01',
    sparkle:'M12 2C10.8 8.2 8.2 10.8 2 12c6.2 1.2 8.8 3.8 10 10 1.2-6.2 3.8-8.8 10-10-6.2-1.2-8.8-3.8-10-10Z',
    money:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 5v14M15 8h-4a2 2 0 0 0 0 4h2a2 2 0 0 1 0 4H9',
    info:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 11v6m0-10h.01',
    checked:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M7 12l3 3 7-7',
    warning:'M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0ZM12 9v4m0 4h.01',
    idle:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',
    thinking:'m12 3-2 6-6 2 6 2 2 6 2-6 6-2-6-2ZM5 2v4M3 4h4M20 17v5m-2.5-2.5h5',
    working:'M3 12h3l2-6 4 13 4-15 2 8h3',
    tool:'M8 5 2 11l6 6m8-12 6 6-6 6m-3-14-2 18',
    complete:'m5 12 4 4L19 6',
    interrupted:'M8 5v14M16 5v14',
    needsInput:'M12 5v9m0 4h.01',
    failed:'m6 6 12 12M6 18 18 6',
    unknown:'M9 8a3 3 0 1 1 5 2c-2 1-2 2-2 4m0 4h.01',
    eye:'M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12M15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0'
  });
  const key=Object.hasOwn(paths,name)?name:'right';
  if (key === 'more') return '<svg class="ui-icon" data-icon="more" viewBox="2 2 20 20" aria-hidden="true"><g fill="currentColor" stroke="none"><circle cx="5" cy="12" r="1.8"/><circle cx="12" cy="12" r="1.8"/><circle cx="19" cy="12" r="1.8"/></g></svg>';
  return `<svg class="ui-icon" data-icon="${key}" viewBox="2 2 20 20" aria-hidden="true"><path d="${paths[key]}"/></svg>`;
};
