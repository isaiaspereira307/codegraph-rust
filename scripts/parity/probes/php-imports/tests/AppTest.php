<?php

namespace Demo\Tests;

use Demo\Helper;
use Demo\{App, Missing as Gone};

require_once __DIR__ . '/../Demo/Helper.php';
require_once __DIR__ . '/../Demo/App.php';

class AppTest
{
    public function testRun(): int
    {
        $app = new \Demo\App();
        return $app->run(new Helper());
    }

    /** @test */
    public function buildsFresh(): int
    {
        $app = new App();
        return $app->build();
    }

    private function helperValue(): int
    {
        return Helper::make()->twice(1);
    }
}
