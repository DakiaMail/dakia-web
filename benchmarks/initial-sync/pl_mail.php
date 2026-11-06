<?php
// Benchmark adapter: real services and persistence, queues drained synchronously.
require '/app/vendor/autoload.php';
(new Symfony\Component\Dotenv\Dotenv())->bootEnv('/app/.env');
class BenchmarkKernel extends App\Kernel {
    public function getProjectDir(): string { return '/app'; }
    public function getCacheDir(): string { return '/app/var/cache/benchmark'; }
    protected function build(Symfony\Component\DependencyInjection\ContainerBuilder $container): void {
        parent::build($container);
        $container->addCompilerPass(new class implements Symfony\Component\DependencyInjection\Compiler\CompilerPassInterface {
            public function process(Symfony\Component\DependencyInjection\ContainerBuilder $c): void {
                foreach ([App\Service\Imap\ImapMailboxImporter::class, Doctrine\ORM\EntityManagerInterface::class] as $id) {
                    $c->setAlias('benchmark.'. $id, $id)->setPublic(true);
                }
            }
        });
    }
}
$kernel = new BenchmarkKernel('prod', false); $kernel->boot();
$c = $kernel->getContainer();
$em = $c->get('benchmark.'. Doctrine\ORM\EntityManagerInterface::class);
if (!preg_match('~@dakia-(?:sync-bench|initial-sync)-db:5432/fixture\?~', getenv('DATABASE_URL') ?: '')) throw new RuntimeException('Disposable benchmark database required');
if (($argv[1] ?? '') === 'setup') {
    if ((int)$em->getConnection()->fetchOne('SELECT COUNT(*) FROM doctrine_migration_versions') === 0) throw new RuntimeException('Run app:db:migrate first');
    $em->getConnection()->executeStatement('TRUNCATE TABLE "user" RESTART IDENTITY CASCADE');
    $u = new App\Entity\User\User(); $u->nameFirst = 'Synthetic'; $u->nameLast = 'Fixture'; $u->email = 'fixture@example.test'; $u->password = 'fixture-unused'; $em->persist($u);
    $a = new App\Entity\Mail\Account(); $a->authType = 'password'; $a->isActive = true; $a->usr = $u; $a->email = 'fixture@example.test'; $a->username = 'fixture'; $a->password = 'fixture-only-password'; $a->imapHost = 'fake-imap'; $a->imapPort = 1993; $a->imapEncryption = 'ssl'; $em->persist($a);
    $m = new App\Entity\Mail\Mailbox(); $m->account = $a; $m->name = 'INBOX'; $m->fullPath = 'INBOX'; $m->delimiter = '/'; $m->isSyncEnabled = true; $em->persist($m); $em->flush();
    echo "setup mailbox={$m->id}\n"; exit;
}
$importer = $c->get('benchmark.'.App\Service\Imap\ImapMailboxImporter::class);
$loop = new App\Infrastructure\ImapClient\ImapLoop();
$endpoint = new App\Infrastructure\ImapClient\ImapEndpoint('fake-imap', 1993, App\Infrastructure\ImapClient\ImapSecurity::Tls, false);
$start = hrtime(true);
$connection = App\Infrastructure\ImapClient\ImapConnection::open($loop, $endpoint);
$connection->authenticate(new App\Infrastructure\ImapClient\ImapCredentials('fixture', 'fixture-only-password'));
$session = new App\Infrastructure\ImapClient\ImapSession($connection);
$pages = 0;
do {
    $outcome = $importer->importPage(1, $session); ++$pages;
    if ($outcome->shouldWait()) throw new RuntimeException('Import did not make progress: '.$outcome->name);
    if ($pages % 100 === 0) fwrite(STDERR, "pages=$pages elapsed=".((hrtime(true)-$start)/1e9)."\n");
} while ($outcome->isUnfinished());
$seconds = (hrtime(true)-$start)/1e9;
$count = $em->getConnection()->fetchOne('SELECT COUNT(*) FROM message');
$verified = $em->getConnection()->fetchAssociative("SELECT COUNT(DISTINCT imap_uid) AS uids, MIN(imap_uid) AS first_uid, MAX(imap_uid) AS last_uid, COUNT(*) FILTER (WHERE strpos(body_text, 'Search token body' || imap_uid || '.') > 0) AS bodies, COUNT(*) FILTER (WHERE search_vector @@ plainto_tsquery('english', 'deterministic mailbox benchmark')) AS indexed, COUNT(*) FILTER (WHERE seen_at IS NULL) AS unread FROM message");
if ((int)$verified['uids'] !== (int)$count || (int)$verified['first_uid'] !== 1 || (int)$verified['last_uid'] !== (int)$count || (int)$verified['bodies'] !== (int)$count || (int)$verified['indexed'] !== (int)$count || (int)$verified['unread'] !== (int)$count) throw new RuntimeException('Body, index or UID validation failed');
echo json_encode(['client'=>'pl_mail','seconds'=>$seconds,'rows'=>(int)$count,'pages'=>$pages,'indexed_bodies'=>(int)$verified['indexed'],'peak_memory_bytes'=>memory_get_peak_usage(true)])."\n";
if ((int)$count !== (int)(getenv('BENCH_COUNT') ?: 50000)) throw new RuntimeException('Wrong persisted row count');
