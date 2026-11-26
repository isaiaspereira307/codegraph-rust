import './helper.dart';
import 'dart:math';

class Base {
  int offset() => 1;
}

mixin Logging {
  void log(String message) {}
}

class App extends Base with Logging {
  int run(Helper helper) => helper.twice(2);

  int build() {
    final fresh = Helper();
    log('build');
    return fresh.twice(3) + this.offset() + max(1, 2);
  }
}
