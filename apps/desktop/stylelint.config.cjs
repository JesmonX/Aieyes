module.exports = {
  plugins: ['./stylelint-design-rules.mjs'],
  rules: {
    'no-duplicate-selectors': true,
    'declaration-block-no-duplicate-properties': [true, { ignore: ['consecutive-duplicates-with-different-syntaxes'] }],
    'color-no-invalid-hex': true,
    'block-no-empty': true,
    'aieyes/design-tokens': true,
  },
};
