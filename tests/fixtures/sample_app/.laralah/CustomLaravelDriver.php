<?php

declare(strict_types=1);

class CustomLaravelDriver
{
    public function canBootstrap(string $projectPath): bool
    {
        return file_exists($projectPath . '/artisan') && file_exists($projectPath . '/bootstrap/app.php');
    }

    public function bootstrap(mixed $app): void
    {
        if (is_object($app) && method_exists($app, 'singleton')) {
            $app->singleton('laralah.driver', 'CustomLaravelDriver');
        }
    }

    public function getAvailableVariables(): array
    {
        return [
            'appName' => 'Laravel Billing Sandbox',
            'appVersion' => 'Laravel 11.34',
        ];
    }
}
