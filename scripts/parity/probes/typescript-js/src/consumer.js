const util = require('./util');
util.helper();

const loaded = import('./util').then((m) => m.helper());
const shared = require('@ws/shared');
shared.shared();

exports.run = function run() {
  helper();
};

Thing.prototype.go = function go() {
  helper();
};

function Thing() {}

if (false) {
  dead();
}

function dead() {}
