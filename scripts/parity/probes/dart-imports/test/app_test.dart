import 'package:probe/app.dart';
import 'package:probe/helper.dart';

void main() {
  test('run', () {
    App().run(Helper());
  });
  test('build', () => App().build());
}
