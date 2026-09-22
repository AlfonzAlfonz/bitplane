/**
 * The three implementation-status admonitions.
 *
 * Each is an ordinary admonition wearing an Infima alert variant, so the
 * colours are the site's own and dark mode is already handled:
 *
 * | directive             | variant           | colour |
 * | --------------------- | ----------------- | ------ |
 * | `:::implemented`      | `alert--success`  | green  |
 * | `:::in-progress`      | `alert--warning`  | yellow |
 * | `:::not-implemented`  | `alert--danger`   | red    |
 *
 * The title is a default rather than a fixed string: a page that wants to say
 * more than "In progress" in the heading writes `:::in-progress[…]`.
 */
import React from 'react';
import clsx from 'clsx';
import AdmonitionLayout from '@theme/Admonition/Layout';
import {
  IconImplemented,
  IconInProgress,
  IconNotImplemented,
} from '@theme/Admonition/Icon/Status';

export const AdmonitionTypeImplemented = statusAdmonition({
  infimaClassName: 'alert alert--success',
  icon: <IconImplemented />,
  title: 'Fully implemented',
});

export const AdmonitionTypeInProgress = statusAdmonition({
  infimaClassName: 'alert alert--warning',
  icon: <IconInProgress />,
  title: 'In progress',
});

export const AdmonitionTypeNotImplemented = statusAdmonition({
  infimaClassName: 'alert alert--danger',
  icon: <IconNotImplemented />,
  title: 'Not implemented',
});

function statusAdmonition({infimaClassName, icon, title}) {
  return function StatusAdmonition(props) {
    return (
      <AdmonitionLayout
        icon={icon}
        title={title}
        {...props}
        className={clsx(infimaClassName, props.className)}>
        {props.children}
      </AdmonitionLayout>
    );
  };
}
