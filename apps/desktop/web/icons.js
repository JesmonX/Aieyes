/* Semantic icons shared by the details window, panel and editors. */
globalThis.uiIcon = (name) => {
  const paths = {switch:'M4 7h16m-4-4 4 4-4 4M20 17H4m4-4-4 4 4 4',credit:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M15 9a4 4 0 1 0 0 6',terminal:'M3 4h18v16H3ZM7 8l3 4-3 4M13 16h4',trend:'M3 3v18h18M6 15l5-5 4 3 6-8',chevron:'m6 9 6 6 6-6',right:'m9 6 6 6-6 6',plus:'M12 5v14M5 12h14',minus:'M5 12h14',x:'m6 6 12 12M18 6 6 18',grip:'M8 5h.01M16 5h.01M8 12h.01M16 12h.01M8 19h.01M16 19h.01',up:'m6 14 6-6 6 6',down:'m6 10 6 6 6-6',sort:'M8 4v16m-4-4 4 4 4-4M16 20V4m-4 4 4-4 4 4',refresh:'M20 11a8 8 0 1 0-2.34 5.66M20 5v6h-6',sources:'M3 7h18v13H3ZM7 7V3h10v4',hosts:'M3 3h18v7H3ZM3 14h18v7H3ZM7 6h.01M7 17h.01',prices:'M12 2v20m5-16H9a4 4 0 0 0 0 8h6a4 4 0 0 1 0 8H6',wakeups:'M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0',general:'M3 7h18M3 17h18M8 4v6M16 14v6',attention:'M12 8v5m0 4h.01M12 3 2 21h20Z'};
  Object.assign(paths, {
    sidebarOpen:'M3 4h18v16H3ZM9 4v16m4-12 4 4-4 4',
    sidebarClose:'M3 4h18v16H3ZM9 4v16m8-12-4 4 4 4',
    pet:'M8 14c-1 2-3 2-3 4s2 3 4 2c2-1 4-1 6 0s4 0 4-2-2-2-3-4c-2-3-6-3-8 0M6 5c-3 0-3 5 0 5S9 5 6 5M11 3c-3 0-3 6 0 6s3-6 0-6M16 4c-3 0-3 5 0 5s3-5 0-5M20 8c-2 0-2 5 0 5s2-5 0-5',
    network:'M12 9v7M5 16h14M5 16v3m14-3v3M8 3h8v6H8ZM2 19h6v3H2Zm14 0h6v3h-6Z',
    sun:'M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.5 1.5m11 11L19 19M5 19l1.5-1.5m11-11L19 5',
    moon:'M20.5 14a8.5 8.5 0 0 1-10.5-10.5A8.5 8.5 0 1 0 20.5 14Z',
    download:'M12 3v12m-5-5 5 5 5-5M5 16v5h14v-5',
    expand:'M14 4h6v6M20 4l-8 8M10 20H4v-6',
    settings:'m9.7 3-.5 2.2-1.7 1-2.2-.7-2.3 4 1.7 1.5v2L3 14.5l2.3 4 2.2-.7 1.7 1 .5 2.2h4.6l.5-2.2 1.7-1 2.2.7 2.3-4-1.7-1.5v-2L21 9.5l-2.3-4-2.2.7-1.7-1-.5-2.2ZM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    more:'M5 12h.01M12 12h.01M19 12h.01',
    sparkle:'M12 2C10.8 8.2 8.2 10.8 2 12c6.2 1.2 8.8 3.8 10 10 1.2-6.2 3.8-8.8 10-10-6.2-1.2-8.8-3.8-10-10Z',
    money:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 5v14M15 8h-4a2 2 0 0 0 0 4h2a2 2 0 0 1 0 4H9',
    info:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 11v6m0-10h.01',
    checked:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M7 12l3 3 7-7',
    warning:'M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0M12 6v7m0 4h.01',
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
  return `<svg class="ui-icon" data-icon="${key}" viewBox="2 2 20 20" aria-hidden="true"><path d="${paths[key]}"/></svg>`;
};
