/**
 * The admonition types this site knows, which is the theme's own set plus the
 * three implementation-status ones.
 *
 * The keywords are registered in `docusaurus.config.js`; this maps each to what
 * renders it. A keyword registered there and missing here renders as `info`
 * with a build-time warning rather than failing, so the two lists are kept in
 * step by hand.
 */
import DefaultAdmonitionTypes from '@theme-original/Admonition/Types';
import {
  AdmonitionTypeImplemented,
  AdmonitionTypeInProgress,
  AdmonitionTypeNotImplemented,
} from '@theme/Admonition/Type/Status';

export default {
  ...DefaultAdmonitionTypes,
  implemented: AdmonitionTypeImplemented,
  'in-progress': AdmonitionTypeInProgress,
  'not-implemented': AdmonitionTypeNotImplemented,
};
