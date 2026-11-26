using Demo;
using Demo.Helpers;

namespace Demo.Tests;

public class AppTest
{
    public void TestRun()
    {
        new App().Run(new Helper());
    }
}
