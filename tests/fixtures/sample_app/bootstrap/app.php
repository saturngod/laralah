<?php

declare(strict_types=1);

use App\Models\Article;
use App\Models\Category;
use App\Models\Link;
use App\Models\Order;
use App\Models\Post;
use App\Models\User;
use Illuminate\Foundation\Application;

$app = new Application(dirname(__DIR__));
$GLOBALS['__tw_laravel_app'] = $app;

User::setSeedRows([
    ['id' => 1, 'name' => 'Taylor Otwell', 'email' => 'taylor@laravel.com', 'plan' => 'enterprise', 'active' => true, 'country' => 'US', 'mrr' => 499.00, 'created_at' => '2024-01-10 09:15:00'],
    ['id' => 2, 'name' => 'Nuno Maduro', 'email' => 'nuno@laravel.com', 'plan' => 'pro', 'active' => true, 'country' => 'PT', 'mrr' => 149.00, 'created_at' => '2024-02-18 14:22:00'],
    ['id' => 3, 'name' => 'Jess Archer', 'email' => 'jess@laravel.com', 'plan' => 'enterprise', 'active' => true, 'country' => 'AU', 'mrr' => 499.00, 'created_at' => '2024-03-04 06:40:00'],
    ['id' => 4, 'name' => 'Joe Dixon', 'email' => 'joe@laravel.com', 'plan' => 'pro', 'active' => true, 'country' => 'UK', 'mrr' => 149.00, 'created_at' => '2024-04-12 11:05:00'],
    ['id' => 5, 'name' => 'Tim MacDonald', 'email' => 'tim@laravel.com', 'plan' => 'starter', 'active' => false, 'country' => 'AU', 'mrr' => 29.00, 'created_at' => '2024-05-20 03:50:00'],
    ['id' => 6, 'name' => 'Christoph Rumpel', 'email' => 'christoph@laravel.dev', 'plan' => 'pro', 'active' => true, 'country' => 'AT', 'mrr' => 149.00, 'created_at' => '2024-06-15 16:12:00'],
    ['id' => 7, 'name' => 'Marcel Pociot', 'email' => 'marcel@beyondco.de', 'plan' => 'enterprise', 'active' => true, 'country' => 'DE', 'mrr' => 499.00, 'created_at' => '2024-07-02 10:30:00'],
    ['id' => 8, 'name' => 'Freek Van der Herten', 'email' => 'freek@spatie.be', 'plan' => 'pro', 'active' => true, 'country' => 'BE', 'mrr' => 149.00, 'created_at' => '2024-08-19 12:45:00'],
]);

Order::setSeedRows([
    ['id' => 101, 'user_id' => 1, 'reference' => 'INV-2026-0101', 'status' => 'paid', 'total' => 499.00, 'currency' => 'USD', 'created_at' => '2026-11-01 10:00:00'],
    ['id' => 102, 'user_id' => 2, 'reference' => 'INV-2026-0102', 'status' => 'paid', 'total' => 149.00, 'currency' => 'USD', 'created_at' => '2026-11-03 11:20:00'],
    ['id' => 103, 'user_id' => 3, 'reference' => 'INV-2026-0103', 'status' => 'shipped', 'total' => 499.00, 'currency' => 'USD', 'created_at' => '2026-11-05 08:45:00'],
    ['id' => 104, 'user_id' => 4, 'reference' => 'INV-2026-0104', 'status' => 'pending', 'total' => 149.00, 'currency' => 'USD', 'created_at' => '2026-11-08 17:10:00'],
    ['id' => 105, 'user_id' => 1, 'reference' => 'INV-2026-0105', 'status' => 'paid', 'total' => 299.00, 'currency' => 'USD', 'created_at' => '2026-11-11 09:30:00'],
    ['id' => 106, 'user_id' => 7, 'reference' => 'INV-2026-0106', 'status' => 'shipped', 'total' => 499.00, 'currency' => 'USD', 'created_at' => '2026-11-14 13:15:00'],
]);

Post::setSeedRows([
    ['id' => 201, 'user_id' => 1, 'title' => 'Laravel 11 Slim Skeleton', 'published' => true, 'views' => 18420],
    ['id' => 202, 'user_id' => 2, 'title' => 'Pest 3 Architecture Testing', 'published' => true, 'views' => 12190],
    ['id' => 203, 'user_id' => 3, 'title' => 'Laravel Pulse Real-time Metrics', 'published' => true, 'views' => 9840],
    ['id' => 204, 'user_id' => 7, 'title' => 'Inspecting Eloquent Chains Live', 'published' => true, 'views' => 15200],
]);

Category::setSeedRows([
    ['id' => 10, 'name' => 'Laravel Packages', 'slug' => 'packages', 'description' => 'Community created packages that add functionality to Laravel', 'created_at' => '2024-09-26 19:54:11'],
    ['id' => 12, 'name' => 'Laravel Tutorials', 'slug' => 'tutorials', 'description' => 'Tutorials on everything from getting started with Laravel to expert topics', 'created_at' => '2024-09-26 19:54:11'],
]);

Article::setSeedRows([
    ['id' => 1, 'user_id' => 1, 'category_id' => 10, 'title' => 'Introducing Laravel Cloud', 'slug' => 'introducing-laravel-cloud', 'published' => true, 'created_at' => '2026-10-19 13:00:00'],
    ['id' => 2, 'user_id' => 2, 'category_id' => 12, 'title' => 'Type Coverage in Pest', 'slug' => 'type-coverage-in-pest', 'published' => true, 'created_at' => '2026-10-19 15:20:00'],
    ['id' => 3, 'user_id' => 3, 'category_id' => 12, 'title' => 'Mastering Eloquent Subqueries', 'slug' => 'mastering-eloquent-subqueries', 'published' => true, 'created_at' => '2026-10-20 09:45:00'],
]);

Link::setSeedRows([
    ['id' => 1, 'user_id' => 1, 'title' => 'Eloquent touch() and $touches: Update Timestamps [VIDEO]', 'url' => 'https://www.youtube.com/watch?v=243NrYTRYvo', 'status' => 'approved'],
    ['id' => 2, 'user_id' => 2, 'title' => 'Laravel 11 Slim Application Structure', 'url' => 'https://laravel.com/docs/structure', 'status' => 'approved'],
]);

return $app;
