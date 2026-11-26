<?php

namespace Demo;

class Helper
{
    public static function make(): self
    {
        return new self();
    }

    public function twice(int $value): int
    {
        return $value * 2;
    }
}
