import React from 'react';
import { helper, Widget, wrap } from './util';

const Node = React.memo(function Node() {
  return <div onClick={() => helper()}><Widget /></div>;
});

const Forwarded = React.forwardRef((props, ref) => <Widget />);

export function renderAll() {
  Node();
  Forwarded();
  wrap(helper);
  if (false) {
    hidden();
  }
  const users = { get() { return 1; } };
  users.get();
}

function hidden() {}
