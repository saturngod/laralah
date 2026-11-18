<?php

declare(strict_types=1);

namespace Illuminate\Contracts\Mail {
    interface Mailable
    {
        public function render(): string;
    }
}

namespace Illuminate\Support {
    use ArrayAccess;
    use ArrayIterator;
    use Countable;
    use IteratorAggregate;
    use JsonSerializable;
    use Traversable;

    class Str
    {
        public static function slug(string $title, string $separator = '-'): string
        {
            $text = strtolower(trim($title));
            $text = preg_replace('/[^a-z0-9]+/', $separator, $text) ?? $text;
            return trim($text, $separator);
        }

        public static function upper(string $value): string
        {
            return strtoupper($value);
        }

        public static function lower(string $value): string
        {
            return strtolower($value);
        }

        public static function limit(string $value, int $limit = 100, string $end = '...'): string
        {
            if (mb_strlen($value) <= $limit) {
                return $value;
            }
            return rtrim(mb_substr($value, 0, $limit)) . $end;
        }

        public static function contains(string $haystack, string|array $needles): bool
        {
            foreach ((array) $needles as $needle) {
                if ($needle !== '' && str_contains($haystack, (string) $needle)) {
                    return true;
                }
            }
            return false;
        }
    }

    class Arr
    {
        public static function get(array $array, string|int|null $key, mixed $default = null): mixed
        {
            if ($key === null) {
                return $array;
            }
            if (array_key_exists($key, $array)) {
                return $array[$key];
            }
            foreach (explode('.', (string) $key) as $segment) {
                if (is_array($array) && array_key_exists($segment, $array)) {
                    $array = $array[$segment];
                } else {
                    return $default;
                }
            }
            return $array;
        }

        public static function pluck(iterable $array, string $value, ?string $key = null): array
        {
            $results = [];
            foreach ($array as $item) {
                $itemVal = is_array($item) ? ($item[$value] ?? null) : ($item->{$value} ?? null);
                if ($key === null) {
                    $results[] = $itemVal;
                } else {
                    $itemKey = is_array($item) ? ($item[$key] ?? null) : ($item->{$key} ?? null);
                    $results[$itemKey] = $itemVal;
                }
            }
            return $results;
        }
    }

    class Carbon extends \DateTimeImmutable implements JsonSerializable
    {
        public static function now(): self
        {
            return new self('now', new \DateTimeZone('UTC'));
        }

        public static function parse(string $time): self
        {
            return new self($time, new \DateTimeZone('UTC'));
        }

        public function subDays(int $days): self
        {
            return $this->modify("-{$days} days");
        }

        public function addDays(int $days): self
        {
            return $this->modify("+{$days} days");
        }

        public function toDateTimeString(): string
        {
            return $this->format('Y-m-d H:i:s');
        }

        public function toDateString(): string
        {
            return $this->format('Y-m-d');
        }

        public function jsonSerialize(): string
        {
            return $this->format(\DateTimeInterface::ATOM);
        }

        public function __toString(): string
        {
            return $this->toDateTimeString();
        }
    }

    class Collection implements ArrayAccess, Countable, IteratorAggregate, JsonSerializable
    {
        protected array $items = [];

        public function __construct(array $items = [])
        {
            $this->items = $items;
        }

        public static function make(array $items = []): self
        {
            return new self($items);
        }

        public function all(): array
        {
            return $this->items;
        }

        public function count(): int
        {
            return count($this->items);
        }

        public function isEmpty(): bool
        {
            return empty($this->items);
        }

        public function first(?callable $callback = null, mixed $default = null): mixed
        {
            if ($callback === null) {
                foreach ($this->items as $item) {
                    return $item;
                }
                return $default;
            }
            foreach ($this->items as $key => $item) {
                if ($callback($item, $key)) {
                    return $item;
                }
            }
            return $default;
        }

        public function last(): mixed
        {
            if (empty($this->items)) {
                return null;
            }
            return end($this->items);
        }

        public function filter(?callable $callback = null): self
        {
            if ($callback === null) {
                return new self(array_filter($this->items));
            }
            return new self(array_filter($this->items, $callback, ARRAY_FILTER_USE_BOTH));
        }

        public function reject(callable $callback): self
        {
            return $this->filter(fn($item, $key) => !$callback($item, $key));
        }

        public function where(string $key, mixed $operator = null, mixed $value = null): self
        {
            if (func_num_args() === 2) {
                $value = $operator;
                $operator = '=';
            }
            return $this->filter(function ($item) use ($key, $operator, $value) {
                $actual = is_array($item) ? ($item[$key] ?? null) : ($item->{$key} ?? null);
                return match ($operator) {
                    '=', '==' => $actual == $value,
                    '===' => $actual === $value,
                    '!=', '<>' => $actual != $value,
                    '>' => $actual > $value,
                    '>=' => $actual >= $value,
                    '<' => $actual < $value,
                    '<=' => $actual <= $value,
                    default => $actual == $value,
                };
            })->values();
        }

        public function map(callable $callback): self
        {
            $keys = array_keys($this->items);
            $items = array_map($callback, $this->items, $keys);
            return new self(array_combine($keys, $items) ?: $items);
        }

        public function pluck(string $value, ?string $key = null): self
        {
            return new self(Arr::pluck($this->items, $value, $key));
        }

        public function sortBy(string|callable $callback, bool $descending = false): self
        {
            $items = $this->items;
            uasort($items, function ($a, $b) use ($callback, $descending) {
                $va = is_callable($callback) ? $callback($a) : (is_array($a) ? ($a[$callback] ?? null) : ($a->{$callback} ?? null));
                $vb = is_callable($callback) ? $callback($b) : (is_array($b) ? ($b[$callback] ?? null) : ($b->{$callback} ?? null));
                $cmp = $va <=> $vb;
                return $descending ? -$cmp : $cmp;
            });
            return new self($items);
        }

        public function sortByDesc(string|callable $callback): self
        {
            return $this->sortBy($callback, true);
        }

        public function groupBy(string|callable $groupBy): self
        {
            $results = [];
            foreach ($this->items as $key => $item) {
                $groupKey = is_callable($groupBy)
                    ? $groupBy($item, $key)
                    : (is_array($item) ? ($item[$groupBy] ?? '') : ($item->{$groupBy} ?? ''));
                $results[(string) $groupKey][] = $item;
            }
            return new self(array_map(fn($group) => new self($group), $results));
        }

        public function take(int $limit): self
        {
            if ($limit < 0) {
                return new self(array_slice($this->items, $limit, abs($limit)));
            }
            return new self(array_slice($this->items, 0, $limit));
        }

        public function values(): self
        {
            return new self(array_values($this->items));
        }

        public function sum(string|callable|null $callback = null): int|float
        {
            $total = 0;
            foreach ($this->items as $key => $item) {
                if ($callback === null) {
                    $total += is_numeric($item) ? $item : 0;
                } elseif (is_callable($callback)) {
                    $total += $callback($item, $key);
                } else {
                    $total += is_array($item) ? ($item[$callback] ?? 0) : ($item->{$callback} ?? 0);
                }
            }
            return $total;
        }

        public function avg(string|callable|null $callback = null): float
        {
            $count = $this->count();
            return $count === 0 ? 0.0 : $this->sum($callback) / $count;
        }

        public function toArray(): array
        {
            return array_map(function ($value) {
                if (is_object($value) && method_exists($value, 'toArray')) {
                    return $value->toArray();
                }
                return $value;
            }, $this->items);
        }

        public function toJson(int $options = 0): string
        {
            return json_encode($this->jsonSerialize(), $options) ?: '[]';
        }

        public function jsonSerialize(): array
        {
            return $this->toArray();
        }

        public function getIterator(): Traversable
        {
            return new ArrayIterator($this->items);
        }

        public function offsetExists(mixed $offset): bool
        {
            return array_key_exists($offset, $this->items);
        }

        public function offsetGet(mixed $offset): mixed
        {
            return $this->items[$offset] ?? null;
        }

        public function offsetSet(mixed $offset, mixed $value): void
        {
            if ($offset === null) {
                $this->items[] = $value;
            } else {
                $this->items[$offset] = $value;
            }
        }

        public function offsetUnset(mixed $offset): void
        {
            unset($this->items[$offset]);
        }

        public function __tw_magic(callable $fn, int $line, string $expression): self
        {
            $t0 = microtime(true);
            $val = $fn($this);
            $ms = (microtime(true) - $t0) * 1000.0;
            if (function_exists('__tw_record_magic')) {
                __tw_record_magic($line, $expression, 'chain', $ms, $val);
            }
            return $this;
        }
    }
}

namespace Illuminate\Support\Facades {
    class DB
    {
        protected static array $listeners = [];
        protected static array $queryLog = [];

        public static function listen(callable $callback): void
        {
            self::$listeners[] = $callback;
        }

        public static function recordQuery(string $sql, array $bindings = [], float $timeMs = 0.25, string $connection = 'sqlite'): void
        {
            $entry = (object) [
                'sql' => $sql,
                'bindings' => $bindings,
                'time' => $timeMs,
                'connectionName' => $connection,
            ];
            self::$queryLog[] = $entry;
            foreach (self::$listeners as $listener) {
                $listener($entry);
            }
        }

        public static function getQueryLog(): array
        {
            return self::$queryLog;
        }

        public static function flushQueryLog(): void
        {
            self::$queryLog = [];
        }

        public static function select(string $sql, array $bindings = []): array
        {
            $t0 = microtime(true);
            $rows = [];
            $ms = max(0.18, (microtime(true) - $t0) * 1000.0);
            self::recordQuery($sql, $bindings, $ms);
            return $rows;
        }
    }

    class Log
    {
        protected static function write(string $level, string $message, array $context = []): void
        {
            $dir = dirname(__DIR__, 2) . '/storage/logs';
            if (!is_dir($dir)) {
                @mkdir($dir, 0777, true);
            }
            $ts = gmdate('Y-m-d H:i:s');
            $ctx = empty($context) ? '' : ' ' . json_encode($context, JSON_UNESCAPED_SLASHES);
            $line = sprintf("[%s] local.%s: %s%s\n", $ts, strtoupper($level), $message, $ctx);
            @file_put_contents($dir . '/laravel.log', $line, FILE_APPEND);
        }

        public static function debug(string $message, array $context = []): void { self::write('DEBUG', $message, $context); }
        public static function info(string $message, array $context = []): void { self::write('INFO', $message, $context); }
        public static function warning(string $message, array $context = []): void { self::write('WARNING', $message, $context); }
        public static function error(string $message, array $context = []): void { self::write('ERROR', $message, $context); }
        public static function critical(string $message, array $context = []): void { self::write('CRITICAL', $message, $context); }
    }

    class Mail
    {
        public static function to(string|array $users): object
        {
            return new class((array) $users) {
                public function __construct(private array $to) {}
                public function send(\Illuminate\Contracts\Mail\Mailable $mailable): \Illuminate\Mail\SentMessage
                {
                    return new \Illuminate\Mail\SentMessage(
                        $mailable->render(),
                        $this->to,
                        $mailable->getSubject()
                    );
                }
            };
        }
    }

    class Http
    {
        public static function get(string $url, array $query = []): object
        {
            return new class($url, $query) {
                public function __construct(private string $url, private array $query) {}
                public function status(): int { return 200; }
                public function ok(): bool { return true; }
                public function json(): array
                {
                    return [
                        'url' => $this->url,
                        'query' => $this->query,
                        'status' => 'ok',
                        'framework' => 'Laravel 11.x',
                    ];
                }
                public function body(): string
                {
                    return json_encode($this->json(), JSON_PRETTY_PRINT) ?: '{}';
                }
            };
        }
    }
}

namespace Illuminate\Mail {
    use Illuminate\Contracts\Mail\Mailable as MailableContract;

    class SentMessage
    {
        public function __construct(
            protected string $htmlBody,
            public array $to = [],
            public string $subject = ''
        ) {}

        public function getHtmlBody(): string
        {
            return $this->htmlBody;
        }
    }

    abstract class Mailable implements MailableContract
    {
        protected string $subjectLine = 'Laravel Notification';
        protected string $fromAddress = 'billing@laravel-app.test';
        protected array $toAddresses = ['customer@example.com'];
        protected array $sections = [];

        public function subject(string $subject): self
        {
            $this->subjectLine = $subject;
            return $this;
        }

        public function from(string $address): self
        {
            $this->fromAddress = $address;
            return $this;
        }

        public function to(string|array $addresses): self
        {
            $this->toAddresses = (array) $addresses;
            return $this;
        }

        public function getSubject(): string { return $this->subjectLine; }
        public function getFrom(): string { return $this->fromAddress; }
        public function getTo(): array { return $this->toAddresses; }
        public function getSections(): array { return $this->sections; }

        abstract public function render(): string;
    }
}

namespace Illuminate\Database\Eloquent {
    use Illuminate\Support\Facades\DB;

    class Collection extends \Illuminate\Support\Collection {}

    class Builder
    {
        protected array $wheres = [];
        protected array $withRelations = [];
        protected ?array $order = null;
        protected ?int $limitCount = null;

        public function __construct(protected string $modelClass) {}

        public function where(string $column, mixed $operator = null, mixed $value = null): self
        {
            if (func_num_args() === 2) {
                $value = $operator;
                $operator = '=';
            }
            $clone = clone $this;
            $clone->wheres[] = [$column, (string) $operator, $value];
            return $clone;
        }

        public function with(string|array $relations): self
        {
            $clone = clone $this;
            $clone->withRelations = array_values(array_unique(array_merge($clone->withRelations, (array) $relations)));
            return $clone;
        }

        public function orderBy(string $column, string $direction = 'asc'): self
        {
            $clone = clone $this;
            $clone->order = [$column, strtolower($direction) === 'desc' ? 'desc' : 'asc'];
            return $clone;
        }

        public function orderByDesc(string $column): self
        {
            return $this->orderBy($column, 'desc');
        }

        public function limit(int $value): self
        {
            $clone = clone $this;
            $clone->limitCount = $value;
            return $clone;
        }

        public function take(int $value): self
        {
            return $this->limit($value);
        }

        public function toSql(): string
        {
            $table = $this->modelClass::tableName();
            $sql = "select * from \"{$table}\"";
            if (!empty($this->wheres)) {
                $clauses = array_map(fn($w) => "\"{$w[0]}\" {$w[1]} ?", $this->wheres);
                $sql .= ' where ' . implode(' and ', $clauses);
            }
            if ($this->order !== null) {
                $sql .= " order by \"{$this->order[0]}\" {$this->order[1]}";
            }
            if ($this->limitCount !== null) {
                $sql .= " limit {$this->limitCount}";
            }
            return $sql;
        }

        public function getBindings(): array
        {
            return array_map(fn($w) => $w[2], $this->wheres);
        }

        protected function filterRows(): array
        {
            $rows = $this->modelClass::seedRows();
            $filtered = array_filter($rows, function ($row) {
                foreach ($this->wheres as [$col, $op, $val]) {
                    $actual = $row[$col] ?? null;
                    $ok = match ($op) {
                        '=', '==' => $actual == $val,
                        '===' => $actual === $val,
                        '!=', '<>' => $actual != $val,
                        '>' => $actual > $val,
                        '>=' => $actual >= $val,
                        '<' => $actual < $val,
                        '<=' => $actual <= $val,
                        'like' => str_contains(strtolower((string) $actual), strtolower(str_replace('%', '', (string) $val))),
                        default => $actual == $val,
                    };
                    if (!$ok) {
                        return false;
                    }
                }
                return true;
            });

            if ($this->order !== null) {
                [$col, $dir] = $this->order;
                usort($filtered, function ($a, $b) use ($col, $dir) {
                    $cmp = ($a[$col] ?? null) <=> ($b[$col] ?? null);
                    return $dir === 'desc' ? -$cmp : $cmp;
                });
            }

            if ($this->limitCount !== null) {
                $filtered = array_slice(array_values($filtered), 0, $this->limitCount);
            }

            return array_values($filtered);
        }

        public function count(): int
        {
            $table = $this->modelClass::tableName();
            $sql = str_replace('select *', 'select count(*) as aggregate', $this->toSql());
            DB::recordQuery($sql, $this->getBindings(), round(0.15 + count($this->wheres) * 0.08, 2));
            return count($this->filterRows());
        }

        public function get(): Collection
        {
            $t0 = microtime(true);
            $rows = $this->filterRows();
            $ms = round(max(0.24, (microtime(true) - $t0) * 1000.0 + 0.21), 2);
            DB::recordQuery($this->toSql(), $this->getBindings(), $ms);

            $models = array_map(function ($attrs) {
                /** @var Model $model */
                $model = new $this->modelClass($attrs);
                if (!empty($this->withRelations)) {
                    $model->loadRelations($this->withRelations);
                }
                return $model;
            }, $rows);

            return new Collection($models);
        }

        public function first(): ?Model
        {
            return $this->limit(1)->get()->first();
        }

        public function find(int|string $id): ?Model
        {
            return $this->where('id', '=', $id)->first();
        }

        public function __tw_magic(callable $fn, int $line, string $expression): self
        {
            $t0 = microtime(true);
            $val = $fn($this);
            $ms = (microtime(true) - $t0) * 1000.0;
            if (function_exists('__tw_record_magic')) {
                __tw_record_magic($line, $expression, 'chain', $ms, $val);
            }
            return $this;
        }
    }

    abstract class Model implements \JsonSerializable
    {
        protected static string $table = '';
        protected static array $seedDataByClass = [];

        protected array $attributes = [];
        protected array $relations = [];
        protected array $hidden = [];

        public function __construct(array $attributes = [])
        {
            $this->attributes = $attributes;
        }

        public static function tableName(): string
        {
            if (static::$table !== '') {
                return static::$table;
            }
            $parts = explode('\\', static::class);
            return strtolower(end($parts)) . 's';
        }

        public static function seedRows(): array
        {
            return self::$seedDataByClass[static::class] ?? [];
        }

        public static function setSeedRows(array $rows): void
        {
            self::$seedDataByClass[static::class] = $rows;
        }

        public static function query(): Builder
        {
            return new Builder(static::class);
        }

        public static function where(string $column, mixed $operator = null, mixed $value = null): Builder
        {
            if (func_num_args() === 2) {
                return static::query()->where($column, '=', $operator);
            }
            return static::query()->where($column, (string) $operator, $value);
        }

        public static function with(string|array $relations): Builder
        {
            return static::query()->with($relations);
        }

        public static function orderBy(string $column, string $direction = 'asc'): Builder
        {
            return static::query()->orderBy($column, $direction);
        }

        public static function orderByDesc(string $column): Builder
        {
            return static::query()->orderByDesc($column);
        }

        public static function __callStatic(string $method, array $arguments): mixed
        {
            return static::query()->{$method}(...$arguments);
        }

        public static function all(): Collection
        {
            return static::query()->get();
        }

        public static function find(int|string $id): ?static
        {
            $res = static::query()->find($id);
            return $res instanceof static ? $res : null;
        }

        public static function count(): int
        {
            return static::query()->count();
        }

        public function getAttributes(): array
        {
            return $this->attributes;
        }

        public function getRelations(): array
        {
            return $this->relations;
        }

        public function loadRelations(array $relations): self
        {
            foreach ($relations as $relation) {
                if (method_exists($this, $relation)) {
                    $this->relations[$relation] = $this->{$relation}();
                }
            }
            return $this;
        }

        public function __get(string $key): mixed
        {
            if (array_key_exists($key, $this->attributes)) {
                return $this->attributes[$key];
            }
            if (array_key_exists($key, $this->relations)) {
                return $this->relations[$key];
            }
            if (method_exists($this, $key)) {
                $loaded = $this->{$key}();
                $this->relations[$key] = $loaded;
                return $loaded;
            }
            return null;
        }

        public function __isset(string $key): bool
        {
            return array_key_exists($key, $this->attributes) || array_key_exists($key, $this->relations);
        }

        public function toArray(): array
        {
            $out = [];
            foreach ($this->attributes as $k => $v) {
                if (!in_array($k, $this->hidden, true)) {
                    $out[$k] = $v;
                }
            }
            foreach ($this->relations as $rel => $val) {
                if ($val instanceof Collection) {
                    $out[$rel] = $val->toArray();
                } elseif ($val instanceof self) {
                    $out[$rel] = $val->toArray();
                } else {
                    $out[$rel] = $val;
                }
            }
            return $out;
        }

        public function jsonSerialize(): array
        {
            return $this->toArray();
        }

        public function __tw_magic(callable $fn, int $line, string $expression): self
        {
            $t0 = microtime(true);
            $val = $fn($this);
            $ms = (microtime(true) - $t0) * 1000.0;
            if (function_exists('__tw_record_magic')) {
                __tw_record_magic($line, $expression, 'chain', $ms, $val);
            }
            return $this;
        }
    }
}

namespace Illuminate\Foundation {
    class Application
    {
        protected array $bindings = [];

        public function __construct(protected string $basePath) {}

        public function version(): string
        {
            return '11.34.0';
        }

        public function basePath(string $path = ''): string
        {
            return $path === '' ? $this->basePath : $this->basePath . '/' . ltrim($path, '/');
        }

        public function storagePath(string $path = ''): string
        {
            return $this->basePath('storage' . ($path !== '' ? '/' . ltrim($path, '/') : ''));
        }

        public function make(string $abstract): mixed
        {
            if (isset($this->bindings[$abstract])) {
                return $this->bindings[$abstract];
            }
            return $this;
        }

        public function singleton(string $abstract, mixed $instance): void
        {
            $this->bindings[$abstract] = $instance;
        }
    }
}

namespace App\Models {
    use Illuminate\Database\Eloquent\Model;
    use Illuminate\Support\Collection;

    class User extends Model
    {
        protected static string $table = 'users';

        public function orders(): Collection
        {
            return Order::where('user_id', '=', $this->attributes['id'] ?? 0)->get();
        }

        public function posts(): Collection
        {
            return Post::where('user_id', '=', $this->attributes['id'] ?? 0)->get();
        }
    }

    class Order extends Model
    {
        protected static string $table = 'orders';

        public function user(): ?User
        {
            return User::find($this->attributes['user_id'] ?? 0);
        }
    }

    class Post extends Model
    {
        protected static string $table = 'posts';

        public function author(): ?User
        {
            return User::find($this->attributes['user_id'] ?? 0);
        }
    }

    class Category extends Model
    {
        protected static string $table = 'categories';
    }

    class Link extends Model
    {
        protected static string $table = 'links';
    }

    class Article extends Model
    {
        protected static string $table = 'articles';

        public function category(): ?Category
        {
            return Category::find($this->attributes['category_id'] ?? 0);
        }

        public function author(): ?User
        {
            return User::find($this->attributes['user_id'] ?? 0);
        }
    }
}

namespace App\Policies {
    use App\Models\Article;
    use App\Models\User;

    class ArticlePolicy
    {
        public function update(User $user, Article $article): bool
        {
            return $user->id === $article->user_id;
        }
    }
}

namespace App\Http\Resources {
    class ArticleResource
    {
        public function __construct(public mixed $resource) {}
    }
}

namespace Database\Factories {
    use App\Models\Article;

    class ArticleFactory
    {
        public static function new(): self
        {
            return new self();
        }

        public function make(array $overrides = []): Article
        {
            return new Article(array_merge([
                'id' => 99,
                'user_id' => 1,
                'title' => 'Draft Article',
                'slug' => 'draft-article',
                'published' => false,
            ], $overrides));
        }
    }
}

namespace Illuminate\Foundation\Providers {
    class ArtisanServiceProvider {}
}

namespace Illuminate\Console\Events {
    class ArtisanStarting {}
}

namespace App\Mail {
    use App\Models\Link;
    use App\Models\Order;
    use App\Models\User;
    use Illuminate\Mail\Mailable;

    class LinkApproved extends Mailable
    {
        public ?string $locale = null;
        public array $from = [];
        public Link $link;

        public function __construct(Link $link)
        {
            $this->link = $link;
            $this->subjectLine = 'Your Link is approved';
            $this->fromAddress = 'hello@laravel-news.test';
        }

        public function render(): string
        {
            $title = htmlspecialchars((string) $this->link->title, ENT_QUOTES);
            $url = htmlspecialchars((string) $this->link->url, ENT_QUOTES);

            return <<<HTML
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Your Link is approved</title>
</head>
<body style="margin: 0; padding: 36px 16px; background-color: #edf2f7; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; color: #3d4852;">
  <div style="text-align: center; margin-bottom: 24px; font-weight: 700; font-size: 16px; color: #1e293b;">
    Laravel Community News
  </div>
  <div style="max-width: 570px; margin: 0 auto; background: #ffffff; border-radius: 2px; padding: 32px; box-shadow: 0 2px 4px rgba(0, 0, 0, 0.04);">
    <h1 style="margin: 0 0 16px; font-size: 18px; font-weight: 700; color: #2d3748;">Your Link is approved</h1>
    <p style="margin: 0 0 16px; font-size: 15px; line-height: 1.6; color: #4a5568;">
      This is a quick note to let you know the link you submitted has now been approved. Hurray! Here is an overview of the link:
    </p>
    <ul style="margin: 0 0 18px; padding-left: 22px; font-size: 15px; line-height: 1.7; color: #4a5568;">
      <li><strong>Title:</strong> {$title}</li>
      <li><strong>URL:</strong> <a href="{$url}" style="color: #2563eb;">{$url}</a></li>
    </ul>
    <p style="margin: 0 0 16px; font-size: 15px; line-height: 1.6; color: #4a5568;">
      To avoid overwhelming readers, your link is queued and will be published later this week and included in this week's <a href="https://laravel-news.com/newsletter" style="color: #2563eb;">newsletter</a>.
    </p>
    <ul style="margin: 0 0 18px; padding-left: 22px; font-size: 15px; line-height: 1.7;">
      <li><a href="https://linkedin.com" style="color: #2563eb;">LinkedIn</a></li>
      <li><a href="https://facebook.com" style="color: #2563eb;">Facebook</a></li>
      <li><a href="https://x.com" style="color: #2563eb;">X</a></li>
      <li><a href="https://mastodon.social" style="color: #2563eb;">Mastodon</a></li>
    </ul>
    <p style="margin: 0 0 16px; font-size: 15px; line-height: 1.6; color: #4a5568;">
      Thanks again and keep being awesome!
    </p>
    <p style="margin: 0; font-size: 15px; color: #4a5568;">Community Editorial Team</p>
  </div>
  <div style="text-align: center; margin-top: 28px; font-size: 12px; color: #a0aec0;">
    &copy; 2026 Laravel Community News. All rights reserved.
  </div>
</body>
</html>
HTML;
        }
    }

    class InvoiceShipped extends Mailable
    {
        public User $user;
        public Order $order;

        public function __construct(User $user, Order $order)
        {
            $this->user = $user;
            $this->order = $order;
            $this->subjectLine = "Invoice #{$order->reference} Shipped";
            $this->fromAddress = 'orders@acme-laravel.test';
            $this->toAddresses = [$user->email];
            $this->sections = [
                'Greeting' => "Hello {$user->name},",
                'Order Summary' => "Order {$order->reference} ({$order->status}) totaling \${$order->total} USD has shipped.",
                'Plan Tier' => "Customer plan: " . strtoupper((string) $user->plan),
                'CTA' => "View Invoice: https://acme-laravel.test/invoices/{$order->reference}",
            ];
        }

        public function render(): string
        {
            $name = htmlspecialchars((string) $this->user->name, ENT_QUOTES);
            $ref = htmlspecialchars((string) $this->order->reference, ENT_QUOTES);
            $total = number_format((float) $this->order->total, 2);
            $status = htmlspecialchars((string) $this->order->status, ENT_QUOTES);
            $plan = htmlspecialchars(strtoupper((string) $this->user->plan), ENT_QUOTES);

            return <<<HTML
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>Invoice #{$ref}</title>
</head>
<body style="font-family: -apple-system, BlinkMacSystemFont, sans-serif; background: #0f172a; color: #e2e8f0; padding: 32px;">
  <div style="max-width: 560px; margin: 0 auto; background: #1e293b; border-radius: 12px; padding: 28px; border: 1px solid #334155;">
    <p style="color: #fb7185; font-weight: 700; letter-spacing: 0.08em; font-size: 12px; margin: 0 0 8px;">ACME LARAVEL BILLING</p>
    <h1 style="margin: 0 0 16px; font-size: 22px; color: #f8fafc;">Invoice #{$ref} Shipped</h1>
    <p style="line-height: 1.6; color: #cbd5e1;">Hello <strong>{$name}</strong>,</p>
    <p style="line-height: 1.6; color: #cbd5e1;">Your order <code>{$ref}</code> has been processed and shipped. Below are your billing details:</p>
    <table style="width: 100%; border-collapse: collapse; margin: 20px 0; background: #0f172a; border-radius: 8px;">
      <tr><td style="padding: 10px 14px; color: #94a3b8;">Customer Plan</td><td style="padding: 10px 14px; text-align: right; color: #38bdf8;">{$plan}</td></tr>
      <tr><td style="padding: 10px 14px; color: #94a3b8;">Order Status</td><td style="padding: 10px 14px; text-align: right; color: #34d399;">{$status}</td></tr>
      <tr><td style="padding: 10px 14px; color: #94a3b8;">Total Charged</td><td style="padding: 10px 14px; text-align: right; font-weight: 700; color: #f8fafc;">\${$total} USD</td></tr>
    </table>
    <a href="https://acme-laravel.test/invoices/{$ref}" style="display: inline-block; background: #f43f5e; color: white; text-decoration: none; padding: 10px 18px; border-radius: 8px; font-weight: 600;">Download PDF Invoice</a>
  </div>
</body>
</html>
HTML;
        }
    }
}

namespace {
    use Illuminate\Support\Carbon;
    use Illuminate\Support\Collection;

    if (!function_exists('collect')) {
        function collect(array $items = []): Collection
        {
            return new Collection($items);
        }
    }

    if (!function_exists('now')) {
        function now(): Carbon
        {
            return Carbon::now();
        }
    }

    if (!function_exists('config')) {
        function config(string $key, mixed $default = null): mixed
        {
            $cfg = [
                'app.name' => 'Laralah Sample App',
                'app.env' => 'local',
                'app.debug' => true,
                'app.url' => 'http://laravel-playground.test',
                'database.default' => 'sqlite',
                'mail.from.address' => 'billing@acme-laravel.test',
            ];
            return $cfg[$key] ?? $default;
        }
    }

    if (!function_exists('env')) {
        function env(string $key, mixed $default = null): mixed
        {
            $val = getenv($key);
            return $val === false ? $default : $val;
        }
    }

    if (!function_exists('app')) {
        function app(?string $abstract = null): mixed
        {
            global $__tw_laravel_app;
            if ($abstract === null) {
                return $__tw_laravel_app;
            }
            return $__tw_laravel_app?->make($abstract);
        }
    }

    if (!function_exists('base_path')) {
        function base_path(string $path = ''): string
        {
            return app()?->basePath($path) ?? (__DIR__ . '/..' . ($path !== '' ? '/' . ltrim($path, '/') : ''));
        }
    }

    if (!function_exists('storage_path')) {
        function storage_path(string $path = ''): string
        {
            return app()?->storagePath($path) ?? (__DIR__ . '/../storage' . ($path !== '' ? '/' . ltrim($path, '/') : ''));
        }
    }

    if (!function_exists('dump')) {
        function dump(mixed ...$vars): mixed
        {
            if (function_exists('__tw_dump')) {
                return __tw_dump(...$vars);
            }
            foreach ($vars as $var) {
                var_dump($var);
            }
            return count($vars) === 1 ? $vars[0] : $vars;
        }
    }

    if (!function_exists('dd')) {
        function dd(mixed ...$vars): never
        {
            if (function_exists('__tw_dd')) {
                __tw_dd(...$vars);
            }
            foreach ($vars as $var) {
                var_dump($var);
            }
            exit(0);
        }
    }
}
