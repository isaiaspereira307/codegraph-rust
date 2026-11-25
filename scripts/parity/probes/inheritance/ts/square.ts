import { Shape } from './shape';

export class Square extends Shape<string> {
  area(): number {
    return 4;
  }
}
