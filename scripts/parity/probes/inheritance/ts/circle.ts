import React from 'react';
import { Shape } from './shape';

export class Circle extends Shape<number> {
  area(): number {
    return 3;
  }
}

export class Widget extends React.Component {
  render() {
    return null;
  }
}
