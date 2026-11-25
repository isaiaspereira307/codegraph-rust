export function helper() {
  return 1;
}

export class Widget {
  render() {
    helper();
  }
}

export const wrap = (fn: () => void) => fn();
