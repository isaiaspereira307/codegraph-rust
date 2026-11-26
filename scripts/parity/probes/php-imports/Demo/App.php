<?php

namespace Demo;

require_once 'Helper.php';

class App
{
    public function run(Helper $helper): int
    {
        return $helper->twice(2);
    }

    public function build(): int
    {
        $helper = Helper::make();
        $fresh = new Helper();
        return $fresh->twice(3) + self::offset();
    }

    public static function offset(): int
    {
        return 1;
    }
}
